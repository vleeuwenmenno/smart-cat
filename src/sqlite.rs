//! SQLite database view: file header facts, the tables and views with row counts, and for
//! each of the first few: columns, indexes and a sample of rows.
//!
//! The file is untrusted. It is opened read-only and `immutable=1`, so SQLite never
//! creates `-wal`, `-shm` or journal files and never writes. Only fixed metadata queries
//! and `SELECT * FROM "name" LIMIT n` run, with the name quoted as an identifier.
//! `query_only` and `trusted_schema=OFF` stay on, and a progress handler interrupts any
//! query that runs too long.
//!
//! The progress handler only runs between virtual machine steps, and one step (a GLOB
//! over megabytes, say) can take many seconds. So the whole inspection runs in a
//! sandboxed helper (`scat --internal-sqlite`, see [`crate::sandbox`]) that is killed
//! after [`HELPER_TIMEOUT`] and has a memory limit.

use std::io::{self, Write};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use rusqlite::types::ValueRef;
use rusqlite::{Connection, OpenFlags};
use unicode_width::UnicodeWidthStr;

use crate::input::{Input, InputKind, Source};
use crate::render::sanitize_str;
use crate::table::{self, thousands};
use crate::util::human_size;

/// Every SQLite 3 file starts with these 16 bytes.
const MAGIC: &[u8; 16] = b"SQLite format 3\0";
/// One query (row count, sample) may run this long.
const QUERY_BUDGET: Duration = Duration::from_secs(2);
/// All queries of one file together may run this long.
const TOTAL_BUDGET: Duration = Duration::from_secs(8);
/// Wall clock limit for the helper: the query budget plus time to start and print.
pub const HELPER_TIMEOUT: Duration = Duration::from_secs(10);
/// Virtual machine steps between two looks at the clock.
const PROGRESS_STEPS: i32 = 1000;
/// Names in the overview list before `… N more`.
const MAX_LISTED: usize = 200;
/// Longest text cell, in characters, before it is cut.
const MAX_CELL_CHARS: usize = 200;
/// Longest column default shown.
const MAX_DEFAULT_CHARS: usize = 30;
/// Most sample rows, whatever the config says.
pub const MAX_SAMPLE_ROWS: usize = 1000;

/// Whether `auto` sends a file to the database view.
pub fn wants(head: &[u8]) -> bool {
    head.starts_with(MAGIC)
}

pub struct Options<'a> {
    pub name: &'a str,
    pub colors: bool,
    pub utf8: bool,
    pub width: usize,
    /// Rows shown per table; 0 shows none.
    pub sample_rows: usize,
    /// Tables shown in detail; 0 shows the overview only.
    pub max_tables: usize,
}

/// Fields of the 100-byte file header.
#[derive(Debug, PartialEq)]
struct Header {
    page_size: u32,
    pages: u32,
    encoding: String,
    user_version: u32,
    application_id: u32,
    wal: bool,
}

fn parse_header(raw: &[u8], file_size: u64) -> Option<Header> {
    if raw.len() < 100 || !raw.starts_with(MAGIC) {
        return None;
    }
    let be16 = |at: usize| u32::from(u16::from_be_bytes([raw[at], raw[at + 1]]));
    let be32 = |at: usize| u32::from_be_bytes([raw[at], raw[at + 1], raw[at + 2], raw[at + 3]]);
    let page_size = match be16(16) {
        1 => 65536,
        n => n,
    };
    let mut pages = be32(28);
    if pages == 0 && page_size > 0 {
        pages = u32::try_from(file_size / u64::from(page_size)).unwrap_or(u32::MAX);
    }
    let encoding = match be32(56) {
        1 => "UTF-8".to_string(),
        2 => "UTF-16le".to_string(),
        3 => "UTF-16be".to_string(),
        0 => "UTF-8".to_string(),
        n => format!("unknown ({n})"),
    };
    Some(Header {
        page_size,
        pages,
        encoding,
        user_version: be32(60),
        application_id: be32(68),
        wal: raw[18] == 2 || raw[19] == 2,
    })
}

/// A table or view from `sqlite_schema`.
struct Object {
    name: String,
    view: bool,
    virtual_table: bool,
}

struct Column {
    name: String,
    kind: String,
    not_null: bool,
    default: Option<String>,
    pk: u32,
}

/// Wall-clock limits for queries, enforced from SQLite's progress handler.
struct Budget {
    start: Instant,
    /// Milliseconds since `start` after which running queries are interrupted.
    deadline: Arc<AtomicU64>,
}

impl Budget {
    fn install(conn: &Connection) -> rusqlite::Result<Self> {
        let start = Instant::now();
        let deadline = Arc::new(AtomicU64::new(0));
        let seen = Arc::clone(&deadline);
        conn.progress_handler(
            PROGRESS_STEPS,
            Some(move || start.elapsed().as_millis() as u64 > seen.load(Ordering::Relaxed)),
        )?;
        let budget = Self { start, deadline };
        budget.arm();
        Ok(budget)
    }

    /// Starts the clock for the next query.
    fn arm(&self) {
        let now = self.start.elapsed();
        let end = (now + QUERY_BUDGET).min(TOTAL_BUDGET.max(now));
        self.deadline
            .store(end.as_millis() as u64, Ordering::Relaxed);
    }

    fn exhausted(&self) -> bool {
        self.start.elapsed() >= TOTAL_BUDGET
    }
}

/// Quotes `name` as an SQL identifier: wrapped in double quotes, inner quotes doubled.
pub fn quote_identifier(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// `file:` URI for `path` with the read-only, immutable query parameters. Every byte
/// outside a safe set is percent-encoded, so `?`, `#` and `%` in names stay literal.
fn uri_for(path: &Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    let absolute = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let mut uri = String::from("file:");
    for &b in absolute.as_os_str().as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b'.' | b'_' | b'-' | b'~' => {
                uri.push(char::from(b));
            }
            _ => uri.push_str(&format!("%{b:02X}")),
        }
    }
    uri.push_str("?mode=ro&immutable=1");
    uri
}

fn open(path: &Path) -> rusqlite::Result<Connection> {
    let conn = Connection::open_with_flags(
        uri_for(path),
        OpenFlags::SQLITE_OPEN_READ_ONLY
            | OpenFlags::SQLITE_OPEN_URI
            | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    conn.busy_timeout(Duration::from_millis(100))?;
    conn.execute_batch("PRAGMA query_only = ON; PRAGMA trusted_schema = OFF;")?;
    Ok(conn)
}

fn cannot_read(err: impl std::fmt::Display) -> io::Error {
    io::Error::other(format!("cannot read database: {err}"))
}

/// Renders the database view by running [`render_file`] in the sandboxed helper.
pub fn render<W: Write>(
    out: &mut W,
    source: &Source,
    input: &mut Input,
    opts: &Options,
) -> io::Result<()> {
    use crate::sandbox;
    let Source::Path(path) = source else {
        return Err(cannot_read("standard input is not a file"));
    };
    if !matches!(input.kind, InputKind::Regular { .. }) {
        return Err(cannot_read("not a regular file"));
    }
    let flag = |b: bool| if b { "1" } else { "0" };
    let mut cmd = sandbox::helper(sandbox::SQLITE_FLAG, [opts.name]);
    cmd.args([
        flag(opts.colors),
        flag(opts.utf8),
        &opts.width.to_string(),
        &opts.sample_rows.to_string(),
        &opts.max_tables.to_string(),
    ]);
    cmd.arg(path);
    let rendered = sandbox::run_with(
        cmd,
        sandbox::Limits {
            timeout: HELPER_TIMEOUT,
            ..sandbox::Limits::default()
        },
    )
    .map_err(|err| match err {
        // The helper's own message already starts with "cannot read database".
        sandbox::RunError::Failed(message) if message.starts_with("cannot read database") => {
            io::Error::other(message)
        }
        other => cannot_read(other),
    })?;
    out.write_all(&rendered)
}

/// Runs the database helper when scat was started as one (`scat --internal-sqlite NAME
/// COLORS UTF8 WIDTH SAMPLE_ROWS MAX_TABLES PATH`); `None` for a normal run.
pub fn internal_main() -> Option<i32> {
    let mut args = std::env::args_os().skip(1);
    if args.next()? != crate::sandbox::SQLITE_FLAG {
        return None;
    }
    crate::sandbox::report_panics("database reader");
    let args: Vec<std::ffi::OsString> = args.collect();
    let text = |i: usize| args.get(i).and_then(|a| a.to_str());
    let number = |i: usize| text(i).and_then(|t| t.parse::<usize>().ok());
    let (
        Some(name),
        Some(colors),
        Some(utf8),
        Some(width),
        Some(sample_rows),
        Some(max_tables),
        Some(path),
    ) = (
        text(0),
        text(1),
        text(2),
        number(3),
        number(4),
        number(5),
        args.get(6),
    )
    else {
        eprintln!("scat: usage error");
        return Some(2);
    };
    let opts = Options {
        name,
        colors: colors == "1",
        utf8: utf8 == "1",
        width,
        sample_rows,
        max_tables,
    };
    let mut buf = Vec::new();
    match render_file(&mut buf, Path::new(path), &opts) {
        Ok(()) => {
            let mut stdout = io::stdout().lock();
            Some(
                if stdout.write_all(&buf).and_then(|()| stdout.flush()).is_ok() {
                    0
                } else {
                    1
                },
            )
        }
        Err(err) => {
            eprintln!("scat: {}", crate::util::io_message(&err));
            Some(2)
        }
    }
}

/// The database view of the file at `path`. Runs in the helper only.
fn render_file<W: Write>(out: &mut W, path: &Path, opts: &Options) -> io::Result<()> {
    let file_size = std::fs::metadata(path)?.len();
    let mut raw = [0u8; 100];
    let read = std::fs::File::open(path).and_then(|mut f| {
        use std::io::Read;
        let mut got = 0;
        while got < raw.len() {
            match f.read(&mut raw[got..])? {
                0 => break,
                n => got += n,
            }
        }
        Ok(got)
    })?;
    let header = parse_header(&raw[..read], file_size)
        .ok_or_else(|| cannot_read("file is not a database"))?;

    let conn = open(path).map_err(cannot_read)?;
    let budget = Budget::install(&conn).map_err(cannot_read)?;
    let objects = schema_objects(&conn).map_err(cannot_read)?;

    let mut wal_note = None;
    if header.wal {
        let mut wal = path.as_os_str().to_owned();
        wal.push("-wal");
        wal_note = Some(match std::fs::metadata(&wal) {
            Ok(m) if m.len() > 0 => format!(
                "WAL (a {} -wal file exists; immutable mode does not read it)",
                human_size(m.len())
            ),
            _ => "WAL".to_string(),
        });
    }
    let journal = wal_note.unwrap_or_else(|| "rollback journal (delete)".to_string());
    let mut rows: Vec<(&str, String)> = vec![
        ("format", "SQLite 3".to_string()),
        (
            "page size",
            format!("{} B", thousands(u64::from(header.page_size))),
        ),
        ("pages", thousands(u64::from(header.pages))),
        ("file size", human_size(file_size)),
        ("encoding", header.encoding.clone()),
        ("journal", journal),
    ];
    if header.user_version != 0 {
        rows.push(("user_version", header.user_version.to_string()));
    }
    if header.application_id != 0 {
        rows.push(("application_id", application_id(header.application_id)));
    }
    let mut buf = Vec::new();
    crate::kv::write_table(&mut buf, opts.name, &rows, opts.colors)?;
    out.write_all(&buf)?;

    let (bold, dim, reset) = if opts.colors {
        ("\x1b[1m", "\x1b[2m", "\x1b[0m")
    } else {
        ("", "", "")
    };
    let ellipsis = if opts.utf8 { "…" } else { "..." };

    // Overview: every table and view with its row count.
    let tables = objects.iter().filter(|o| !o.view).count();
    let views = objects.len() - tables;
    if objects.is_empty() {
        writeln!(out, "\n{dim}no tables{reset}")?;
        return Ok(());
    }
    writeln!(
        out,
        "\n{bold}Tables{reset} {dim}({}){reset}",
        count_label(tables, views)
    )?;
    let name_width = objects
        .iter()
        .take(MAX_LISTED)
        .map(|o| sanitize_str(&o.name).width())
        .max()
        .unwrap_or(0);
    let mut counts: Vec<Option<u64>> = Vec::with_capacity(objects.len());
    for (index, object) in objects.iter().enumerate() {
        let count = if index < MAX_LISTED && !object.virtual_table && !budget.exhausted() {
            row_count(&conn, &budget, &object.name)
        } else {
            None
        };
        counts.push(count);
    }
    for (object, count) in objects.iter().zip(&counts).take(MAX_LISTED) {
        let name = sanitize_str(&object.name);
        let pad = " ".repeat(name_width.saturating_sub(name.width()));
        writeln!(
            out,
            "  {name}{pad}  {dim}{:<7}{reset}{}",
            kind_label(object),
            count_text(object, *count)
        )?;
    }
    if objects.len() > MAX_LISTED {
        writeln!(
            out,
            "  {dim}{ellipsis} {} more{reset}",
            thousands((objects.len() - MAX_LISTED) as u64)
        )?;
    }

    // Detail for the first tables.
    let shown = objects.len().min(opts.max_tables);
    for (object, count) in objects.iter().zip(&counts).take(shown) {
        writeln!(out)?;
        let heading = format!(
            "{bold}{}{reset}  {dim}{} {}{reset}",
            sanitize_str(&object.name),
            kind_label(object),
            count_text(object, *count),
        );
        writeln!(out, "{heading}")?;
        write_details(out, &conn, &budget, object, *count, opts)?;
    }
    if objects.len() > shown {
        let more = objects.len() - shown;
        writeln!(
            out,
            "\n{dim}{ellipsis} {more} more {}{reset}",
            if more == 1 { "table" } else { "tables" }
        )?;
    }
    Ok(())
}

fn count_label(tables: usize, views: usize) -> String {
    let plural = |n: usize, word: &str| format!("{n} {word}{}", if n == 1 { "" } else { "s" });
    if views == 0 {
        plural(tables, "table")
    } else {
        format!("{}, {}", plural(tables, "table"), plural(views, "view"))
    }
}

fn kind_label(object: &Object) -> &'static str {
    if object.view {
        "view"
    } else if object.virtual_table {
        "virtual"
    } else {
        "table"
    }
}

fn count_text(object: &Object, count: Option<u64>) -> String {
    match count {
        Some(1) => "1 row".to_string(),
        Some(n) => format!("{} rows", thousands(n)),
        None if object.virtual_table => "rows not read".to_string(),
        None => "? rows".to_string(),
    }
}

/// Well-known `application_id` values, else the number (and its ASCII, if it is text).
fn application_id(id: u32) -> String {
    let bytes = id.to_be_bytes();
    let known = match id {
        0x0F05_5112 => Some("Fossil"),
        0x4D50_4435 => Some("MPEG-4 (MP4 metadata)"),
        0x4750_4B47 => Some("GeoPackage"),
        _ => None,
    };
    if let Some(name) = known {
        return format!("{id} ({name})");
    }
    if bytes.iter().all(|b| b.is_ascii_graphic()) {
        return format!("{id} ({})", String::from_utf8_lossy(&bytes));
    }
    id.to_string()
}

/// Tables and views in creation order, tables first. Internal `sqlite_*` tables are left out.
fn schema_objects(conn: &Connection) -> rusqlite::Result<Vec<Object>> {
    let mut stmt = conn.prepare(
        "SELECT type, name, sql FROM sqlite_schema \
         WHERE type IN ('table', 'view') ORDER BY rowid",
    )?;
    let mut objects = Vec::new();
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let kind: String = row.get(0)?;
        let name = lossy(row.get_ref(1)?);
        if name.starts_with("sqlite_") || name.contains('\0') {
            continue;
        }
        let sql = row.get_ref(2).map(lossy).unwrap_or_default();
        let head = sql.trim_start().to_ascii_uppercase();
        objects.push(Object {
            name,
            view: kind == "view",
            virtual_table: kind == "table" && head.starts_with("CREATE VIRTUAL"),
        });
    }
    objects.sort_by_key(|o| o.view);
    Ok(objects)
}

/// `Some` count, or `None` when the query failed or ran out of time.
fn row_count(conn: &Connection, budget: &Budget, name: &str) -> Option<u64> {
    budget.arm();
    let sql = format!("SELECT count(*) FROM {}", quote_identifier(name));
    conn.query_row(&sql, [], |row| row.get::<_, i64>(0))
        .ok()
        .and_then(|n| u64::try_from(n).ok())
}

fn lossy(value: ValueRef) -> String {
    match value {
        ValueRef::Text(bytes) | ValueRef::Blob(bytes) => {
            String::from_utf8_lossy(bytes).into_owned()
        }
        ValueRef::Integer(n) => n.to_string(),
        ValueRef::Real(f) => f.to_string(),
        ValueRef::Null => String::new(),
    }
}

fn write_details<W: Write>(
    out: &mut W,
    conn: &Connection,
    budget: &Budget,
    object: &Object,
    count: Option<u64>,
    opts: &Options,
) -> io::Result<()> {
    let dim = if opts.colors { "\x1b[2m" } else { "" };
    let reset = if opts.colors { "\x1b[0m" } else { "" };
    if object.virtual_table {
        return Ok(());
    }
    let columns = match columns(conn, &object.name) {
        Ok(columns) => columns,
        Err(err) => {
            writeln!(
                out,
                "  {dim}schema unreadable: {}{reset}",
                sanitize_str(&err.to_string())
            )?;
            return Ok(());
        }
    };
    let schema: Vec<String> = columns.iter().map(describe_column).collect();
    write_wrapped(out, "columns", &schema, opts, dim, reset)?;
    if !object.view {
        match indexes(conn, &object.name) {
            Ok(list) if !list.is_empty() => {
                write_wrapped(out, "indexes", &list, opts, dim, reset)?;
            }
            Ok(_) => {}
            Err(err) => writeln!(
                out,
                "  {dim}indexes unreadable: {}{reset}",
                sanitize_str(&err.to_string())
            )?,
        }
    }
    if opts.sample_rows == 0 || columns.is_empty() || budget.exhausted() {
        return Ok(());
    }
    match sample(conn, budget, object, &columns, opts.sample_rows) {
        Ok(rows) => {
            let shown = (rows.len() - 1) as u64;
            if shown == 0 {
                writeln!(out, "  {dim}empty{reset}")?;
                return Ok(());
            }
            let more = count.map_or(0, |total| total.saturating_sub(shown));
            let table_opts = table::Options {
                width: opts.width,
                colors: opts.colors,
                utf8: opts.utf8,
                line_numbers: false,
                max_rows: 0,
            };
            table::render_rows(&rows, more, &table_opts, out)?;
        }
        Err(err) => writeln!(
            out,
            "  {dim}rows unreadable: {}{reset}",
            sanitize_str(&err.to_string())
        )?,
    }
    Ok(())
}

fn columns(conn: &Connection, table: &str) -> rusqlite::Result<Vec<Column>> {
    let mut stmt = conn.prepare(
        "SELECT name, type, \"notnull\", dflt_value, pk FROM pragma_table_info(?1) ORDER BY cid",
    )?;
    let mut rows = stmt.query([table])?;
    let mut columns = Vec::new();
    while let Some(row) = rows.next()? {
        columns.push(Column {
            name: lossy(row.get_ref(0)?),
            kind: lossy(row.get_ref(1)?),
            not_null: row.get::<_, i64>(2)? != 0,
            default: match row.get_ref(3)? {
                ValueRef::Null => None,
                other => Some(lossy(other)),
            },
            pk: u32::try_from(row.get::<_, i64>(4)?).unwrap_or(0),
        });
    }
    Ok(columns)
}

fn describe_column(column: &Column) -> String {
    let mut text = sanitize_str(&column.name);
    if !column.kind.is_empty() {
        text.push(' ');
        text.push_str(&sanitize_str(&column.kind));
    }
    if column.pk > 0 {
        text.push_str(" PK");
    }
    if column.not_null {
        text.push_str(" NOT NULL");
    }
    if let Some(default) = &column.default {
        let flat = sanitize_str(&default.replace(['\n', '\r'], " "));
        let shortened: String = flat.chars().take(MAX_DEFAULT_CHARS).collect();
        let dots = if flat.chars().count() > MAX_DEFAULT_CHARS {
            "…"
        } else {
            ""
        };
        text.push_str(&format!(" DEFAULT {shortened}{dots}"));
    }
    text
}

/// Created and unique-constraint indexes of `table`, as `name (cols)` with `UNIQUE`.
fn indexes(conn: &Connection, table: &str) -> rusqlite::Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT name, \"unique\", origin, partial FROM pragma_index_list(?1) ORDER BY seq",
    )?;
    let mut found = Vec::new();
    let mut rows = stmt.query([table])?;
    while let Some(row) = rows.next()? {
        let name = lossy(row.get_ref(0)?);
        let unique = row.get::<_, i64>(1)? != 0;
        let origin = lossy(row.get_ref(2)?);
        let partial = row.get::<_, i64>(3)? != 0;
        if origin == "pk" {
            continue;
        }
        found.push((name, unique, partial));
    }
    drop(rows);
    drop(stmt);
    let mut described = Vec::new();
    let mut by_column = conn.prepare("SELECT name FROM pragma_index_info(?1) ORDER BY seqno")?;
    for (name, unique, partial) in found {
        let mut cols = Vec::new();
        let mut rows = by_column.query([&name])?;
        while let Some(row) = rows.next()? {
            // Expression indexes have no column name.
            cols.push(match row.get_ref(0)? {
                ValueRef::Null => "expr".to_string(),
                other => sanitize_str(&lossy(other)),
            });
        }
        let mut text = String::new();
        if unique {
            text.push_str("UNIQUE ");
        }
        text.push_str(&format!("{} ({})", sanitize_str(&name), cols.join(", ")));
        if partial {
            text.push_str(" WHERE …");
        }
        described.push(text);
    }
    Ok(described)
}

/// Header row plus up to `limit` data rows, every cell already flattened to text.
fn sample(
    conn: &Connection,
    budget: &Budget,
    object: &Object,
    columns: &[Column],
    limit: usize,
) -> rusqlite::Result<Vec<Vec<String>>> {
    budget.arm();
    let limit = limit.min(MAX_SAMPLE_ROWS);
    let sql = format!(
        "SELECT * FROM {} LIMIT {limit}",
        quote_identifier(&object.name)
    );
    let mut stmt = conn.prepare(&sql)?;
    let width = stmt.column_count();
    let mut rows = vec![columns.iter().map(|c| c.name.clone()).collect::<Vec<_>>()];
    // Hidden columns can make the header shorter than the data; pad it.
    rows[0].resize(width, String::new());
    let mut cursor = stmt.query([])?;
    while let Some(row) = cursor.next()? {
        let mut cells = Vec::with_capacity(width);
        for i in 0..width {
            cells.push(cell_text(row.get_ref(i)?));
        }
        rows.push(cells);
    }
    Ok(rows)
}

fn cell_text(value: ValueRef) -> String {
    match value {
        ValueRef::Null => "NULL".to_string(),
        ValueRef::Integer(n) => n.to_string(),
        ValueRef::Real(f) => f.to_string(),
        ValueRef::Blob(bytes) => format!("<blob {} B>", thousands(bytes.len() as u64)),
        ValueRef::Text(bytes) => {
            let text = String::from_utf8_lossy(bytes);
            if text.chars().nth(MAX_CELL_CHARS).is_some() {
                let cut: String = text.chars().take(MAX_CELL_CHARS).collect();
                format!("{cut}…")
            } else {
                text.into_owned()
            }
        }
    }
}

/// `label  item, item, ...` wrapped at the terminal width, continuation lines aligned.
fn write_wrapped<W: Write>(
    out: &mut W,
    label: &str,
    items: &[String],
    opts: &Options,
    dim: &str,
    reset: &str,
) -> io::Result<()> {
    let indent = 2 + 8 + 2;
    let room = opts.width.saturating_sub(indent).max(20);
    let mut lines: Vec<String> = Vec::new();
    let mut line = String::new();
    for (i, item) in items.iter().enumerate() {
        let piece = if i + 1 < items.len() {
            format!("{item}, ")
        } else {
            item.clone()
        };
        if !line.is_empty() && line.width() + piece.width() > room {
            lines.push(std::mem::take(&mut line).trim_end().to_string());
        }
        line.push_str(&piece);
    }
    lines.push(line);
    for (i, line) in lines.iter().enumerate() {
        let tag = if i == 0 { label } else { "" };
        writeln!(out, "  {dim}{tag:<8}{reset}  {line}")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifiers_are_quoted() {
        assert_eq!(quote_identifier("users"), "\"users\"");
        assert_eq!(quote_identifier("a\"b; DROP"), "\"a\"\"b; DROP\"");
    }

    #[test]
    fn uri_encodes_unsafe_bytes() {
        let uri = uri_for(Path::new("/nonexistent/a b?c#d%.db"));
        assert_eq!(
            uri,
            "file:/nonexistent/a%20b%3Fc%23d%25.db?mode=ro&immutable=1"
        );
    }

    #[test]
    fn header_fields() {
        let mut raw = vec![0u8; 100];
        raw[..16].copy_from_slice(MAGIC);
        raw[16..18].copy_from_slice(&4096u16.to_be_bytes());
        raw[18] = 2;
        raw[19] = 2;
        raw[28..32].copy_from_slice(&7u32.to_be_bytes());
        raw[56..60].copy_from_slice(&1u32.to_be_bytes());
        raw[60..64].copy_from_slice(&5u32.to_be_bytes());
        let header = parse_header(&raw, 28672).unwrap();
        assert_eq!(header.page_size, 4096);
        assert_eq!(header.pages, 7);
        assert!(header.wal);
        assert_eq!(header.user_version, 5);
        assert!(parse_header(&raw[..50], 50).is_none());
    }
}
