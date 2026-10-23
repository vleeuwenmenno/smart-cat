//! PDF view: an info header, then pages drawn as images, or their text.
//!
//! Parsing happens in a sandboxed child process (see [`sandbox`]): hayro, re-run from
//! scat's own binary, is the built-in parser and renderer. When it fails on a file or
//! page, poppler's `pdfinfo` / `pdftoppm` take over if they are installed. Text comes
//! from `pdftotext -layout`; hayro has no text extraction.

mod helper;
mod pages;
pub mod sandbox;

use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};

use crate::image::{Screen, ShowOptions};
use crate::input::{Input, InputKind, Source};
use crate::util::{human_size, which};

pub use pages::PageSpec;

/// Largest non-file input (standard input, a pipe) copied to a temporary file.
const MAX_STREAM_BYTES: u64 = 512 * 1024 * 1024;

/// What the helper (or `pdfinfo`) reports about a document. Strings come from the
/// file and are sanitized when shown.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Info {
    pub pages: u32,
    /// Size of the first page in points, rotation applied.
    pub page_size: Option<(f32, f32)>,
    pub version: Option<String>,
    pub title: Option<String>,
    pub author: Option<String>,
    pub subject: Option<String>,
    pub creator: Option<String>,
    pub producer: Option<String>,
    pub created: Option<String>,
    pub modified: Option<String>,
    pub encrypted: bool,
    pub needs_password: bool,
    /// Why the document could not be opened.
    pub error: Option<String>,
}

/// Runs the helper when scat was started as one; `None` for a normal run.
pub fn internal_main() -> Option<i32> {
    let mut args = std::env::args_os().skip(1);
    if args.next()? != sandbox::FLAG {
        return None;
    }
    Some(helper::main(args.collect()))
}

/// Settings for [`render`].
pub struct Options<'a> {
    /// Name for the header and messages (already sanitized).
    pub name: &'a str,
    pub pages: &'a PageSpec,
    /// Most pages drawn as images per file.
    pub max_pages: u32,
    /// Draw pages with this screen; `None` shows their text.
    pub screen: Option<Screen>,
    pub colors: bool,
    pub utf8: bool,
    /// No header and footer.
    pub plain: bool,
}

/// Renders a PDF: header, selected pages, a footer that says how to see more.
pub fn render<W: Write>(
    out: &mut W,
    source: &Source,
    input: &mut Input,
    opts: &Options,
) -> io::Result<()> {
    // The helpers open the file by name; anything else goes through a private copy.
    let temp;
    let (path, size): (&Path, Option<u64>) = match (source, input.kind) {
        (Source::Path(path), InputKind::Regular { size }) => (path, Some(size)),
        _ => {
            temp = TempCopy::new(&mut input.reader)?;
            (&temp.path, Some(temp.size))
        }
    };

    let info = read_info(path)?;
    let mut header_lines = 0;
    if !opts.plain {
        let mut buf = Vec::new();
        write_header(&mut buf, opts.name, &info, size, opts.colors)?;
        buf.push(b'\n');
        header_lines = buf.iter().filter(|&&b| b == b'\n').count();
        out.write_all(&buf)?;
    }
    if info.needs_password {
        out.flush()?;
        return Err(io::Error::other(
            "encrypted PDF: a password is needed, pages not shown",
        ));
    }
    if info.pages == 0 {
        return Err(io::Error::other("PDF has no pages"));
    }

    let (selected, missing) = opts.pages.resolve(info.pages);
    if let Some(page) = missing {
        out.flush()?;
        eprintln!(
            "scat: {}: no page {page} (the document has {})",
            opts.name,
            plural(info.pages, "page")
        );
    }
    if selected.is_empty() {
        return Ok(());
    }

    let (shown, failures) = match &opts.screen {
        Some(screen) => {
            let limit = (opts.max_pages.max(1) as usize).min(selected.len());
            let shown = &selected[..limit];
            let failures = draw_pages(out, path, shown, screen, header_lines, opts)?;
            if shown.len() < selected.len() && !opts.plain {
                note(
                    out,
                    opts.colors,
                    &format!(
                        "showing {} of {} selected pages; [pdf] max_pages sets the limit",
                        shown.len(),
                        selected.len()
                    ),
                )?;
            }
            (shown.to_vec(), failures)
        }
        None => {
            write_text(out, path, &selected, info.pages, opts)?;
            (selected.clone(), Vec::new())
        }
    };

    if !opts.plain
        && let Some(footer) = footer(&shown, info.pages, opts.utf8)
    {
        if opts.screen.is_none() {
            out.write_all(b"\n")?;
        }
        note(out, opts.colors, &footer)?;
    }
    match failures.as_slice() {
        [] => Ok(()),
        [(page, why)] => Err(io::Error::other(format!(
            "page {page}: cannot render: {why}"
        ))),
        [(page, why), rest @ ..] => Err(io::Error::other(format!(
            "page {page}: cannot render: {why} (and {} more)",
            plural(rest.len() as u32, "page")
        ))),
    }
}

/// Info from the hayro helper, else from `pdfinfo`. An unreadable file is an error.
fn read_info(path: &Path) -> io::Result<Info> {
    let hayro = sandbox::run(sandbox::helper([
        std::ffi::OsStr::new("info"),
        path.as_os_str(),
    ]))
    .map_err(|e| e.to_string())
    .and_then(|bytes| serde_json::from_slice::<Info>(&bytes).map_err(|e| e.to_string()));
    let hayro_error = match hayro {
        Ok(info) if info.error.is_none() || info.needs_password => return Ok(info),
        Ok(info) => info.error.unwrap_or_default(),
        Err(err) => err,
    };
    if let Some(info) = pdfinfo(path) {
        return Ok(info);
    }
    Err(io::Error::other(format!("cannot read PDF: {hayro_error}")))
}

fn pdfinfo(path: &Path) -> Option<Info> {
    let program = which("pdfinfo")?;
    let mut cmd = Command::new(program);
    cmd.args(["-enc", "UTF-8", "--"]).arg(path);
    let bytes = sandbox::run(cmd).ok()?;
    let info = parse_pdfinfo(&String::from_utf8_lossy(&bytes));
    (info.pages > 0).then_some(info)
}

fn parse_pdfinfo(text: &str) -> Info {
    let mut info = Info::default();
    for line in text.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        let field = || (!value.is_empty()).then(|| value.chars().take(300).collect());
        match key {
            "Title" => info.title = field(),
            "Author" => info.author = field(),
            "Subject" => info.subject = field(),
            "Creator" => info.creator = field(),
            "Producer" => info.producer = field(),
            "CreationDate" => info.created = field(),
            "ModDate" => info.modified = field(),
            "Pages" => info.pages = value.parse().unwrap_or(0),
            "Encrypted" => info.encrypted = value.starts_with("yes"),
            "PDF version" => info.version = field(),
            "Page size" => {
                // "612 x 792 pts (letter)"
                let mut parts = value.split_whitespace();
                let w = parts.next().and_then(|v| v.parse().ok());
                let h = parts.nth(1).and_then(|v| v.parse().ok());
                if let (Some(w), Some(h)) = (w, h) {
                    info.page_size = Some((w, h));
                }
            }
            _ => {}
        }
    }
    info
}

fn write_header<W: Write>(
    out: &mut W,
    name: &str,
    info: &Info,
    size: Option<u64>,
    colors: bool,
) -> io::Result<()> {
    let text = |v: &Option<String>| v.clone().unwrap_or_default();
    let mut pdf = info
        .version
        .as_ref()
        .map(|v| format!("PDF {v}"))
        .unwrap_or_else(|| "PDF".into());
    if info.needs_password {
        pdf.push_str(", encrypted (needs a password)");
    } else if info.encrypted {
        pdf.push_str(", encrypted (opened without a password)");
    }
    if let Some(size) = size {
        pdf.push_str(&format!(", {}", human_size(size)));
    }
    let producer = match (&info.creator, &info.producer) {
        (Some(c), Some(p)) if c != p => format!("{p} (created with {c})"),
        (c, p) => p.clone().or_else(|| c.clone()).unwrap_or_default(),
    };
    let rows = [
        ("title", text(&info.title)),
        ("author", text(&info.author)),
        ("subject", text(&info.subject)),
        (
            "pages",
            if info.pages > 0 {
                info.pages.to_string()
            } else {
                String::new()
            },
        ),
        (
            "page size",
            info.page_size.map(page_size).unwrap_or_default(),
        ),
        ("created", text(&info.created)),
        ("producer", producer),
        ("file", pdf),
    ];
    crate::kv::write_table(out, name, &rows, colors)
}

/// "210 x 297 mm (A4)" from a size in points.
fn page_size((w, h): (f32, f32)) -> String {
    const NAMED: &[(&str, f32, f32)] = &[
        ("A3", 842.0, 1191.0),
        ("A4", 595.0, 842.0),
        ("A5", 420.0, 595.0),
        ("Letter", 612.0, 792.0),
        ("Legal", 612.0, 1008.0),
        ("Tabloid", 792.0, 1224.0),
    ];
    let mm = |pt: f32| (pt * 25.4 / 72.0).round();
    let mut text = format!("{} x {} mm", mm(w), mm(h));
    let close = |a: f32, b: f32| (a - b).abs() < 3.0;
    for &(name, nw, nh) in NAMED {
        if close(w, nw) && close(h, nh) {
            text.push_str(&format!(" ({name})"));
        } else if close(w, nh) && close(h, nw) {
            text.push_str(&format!(" ({name} landscape)"));
        }
    }
    text
}

/// Draws each page; returns the pages that could not be rendered and why.
fn draw_pages<W: Write>(
    out: &mut W,
    path: &Path,
    pages: &[u32],
    screen: &Screen,
    header_lines: usize,
    opts: &Options,
) -> io::Result<Vec<(u32, String)>> {
    // Fit header, page and footer on one screen when the terminal is tall enough.
    let reserved = header_lines as u16 + 2;
    let max_rows = screen
        .rows
        .saturating_sub(reserved)
        .clamp(8.min(screen.max_rows), screen.max_rows.max(1));
    let box_px = (
        u32::from(screen.cols) * u32::from(screen.cell.0),
        u32::from(max_rows) * u32::from(screen.cell.1),
    );
    let mut failures = Vec::new();
    for (i, &page) in pages.iter().enumerate() {
        out.flush()?;
        let png = match render_page(path, page, box_px) {
            Ok(png) => png,
            Err(why) => {
                failures.push((page, why));
                continue;
            }
        };
        let img = match image::load_from_memory_with_format(&png, image::ImageFormat::Png) {
            Ok(img) => img,
            Err(err) => {
                failures.push((page, err.to_string()));
                continue;
            }
        };
        if i > 0 {
            out.write_all(b"\n")?;
        }
        let caption = format!("page {page}");
        crate::image::show(
            out,
            screen,
            &img,
            &ShowOptions {
                max_rows: Some(max_rows),
                caption: (pages.len() > 1 && !opts.plain).then_some(caption.as_str()),
                ..Default::default()
            },
        )?;
    }
    Ok(failures)
}

/// A PNG of one page that fits in `box_px`: hayro first, then `pdftoppm`.
pub fn render_page(path: &Path, page: u32, box_px: (u32, u32)) -> Result<Vec<u8>, String> {
    let page_arg = page.to_string();
    let w = box_px.0.max(16).to_string();
    let h = box_px.1.max(16).to_string();
    let hayro = sandbox::run(sandbox::helper([
        std::ffi::OsStr::new("render"),
        path.as_os_str(),
        page_arg.as_ref(),
        w.as_ref(),
        h.as_ref(),
    ]));
    let hayro_error = match hayro {
        Ok(png) if !png.is_empty() => return Ok(png),
        Ok(_) => "no output".to_string(),
        Err(err) => err.to_string(),
    };
    let Some(program) = which("pdftoppm") else {
        return Err(hayro_error);
    };
    let mut cmd = Command::new(program);
    cmd.args([
        "-q",
        "-png",
        "-singlefile",
        "-f",
        &page_arg,
        "-l",
        &page_arg,
    ])
    .args(["-scale-to-x", &w, "-scale-to-y", "-1", "--"])
    .arg(path);
    match sandbox::run(cmd) {
        Ok(png) if !png.is_empty() => Ok(png),
        Ok(_) => Err(format!("{hayro_error}; pdftoppm: no output")),
        Err(err) => Err(format!("{hayro_error}; pdftoppm: {err}")),
    }
}

/// Text of the selected pages with `pdftotext -layout`, sanitized.
fn write_text<W: Write>(
    out: &mut W,
    path: &Path,
    pages: &[u32],
    total: u32,
    opts: &Options,
) -> io::Result<()> {
    let Some(program) = which("pdftotext") else {
        out.flush()?;
        return Err(io::Error::other(
            "no text output: install poppler (pdftotext) for text output",
        ));
    };
    let mut first = true;
    for (start, end) in runs(pages) {
        let mut cmd = Command::new(&program);
        cmd.args(["-q", "-layout", "-enc", "UTF-8"])
            .args(["-f", &start.to_string(), "-l", &end.to_string(), "--"])
            .arg(path)
            .arg("-");
        let bytes =
            sandbox::run(cmd).map_err(|err| io::Error::other(format!("pdftotext: {err}")))?;
        let text = String::from_utf8_lossy(&bytes);
        for (offset, page_text) in text.split('\x0c').enumerate() {
            let page = start + offset as u32;
            if page > end {
                break;
            }
            if !opts.plain && pages.len() > 1 {
                if !first {
                    out.write_all(b"\n")?;
                }
                note(out, opts.colors, &format!("page {page} of {total}"))?;
            }
            first = false;
            let lines: Vec<&str> = page_text.lines().collect();
            let end_at = lines
                .iter()
                .rposition(|l| !l.trim().is_empty())
                .map_or(0, |p| p + 1);
            for line in &lines[..end_at] {
                writeln!(out, "{}", crate::render::sanitize_line(line, 8))?;
            }
        }
    }
    Ok(())
}

/// Splits a page list into runs of consecutive pages.
fn runs(pages: &[u32]) -> Vec<(u32, u32)> {
    let mut runs: Vec<(u32, u32)> = Vec::new();
    for &page in pages {
        match runs.last_mut() {
            Some((_, end)) if *end + 1 == page => *end = page,
            _ => runs.push((page, page)),
        }
    }
    runs
}

/// "page 1 of 12 — use --pages 2-4 for more", or `None` when every page was shown.
fn footer(shown: &[u32], total: u32, utf8: bool) -> Option<String> {
    if shown.len() as u32 >= total {
        return None;
    }
    let dash = if utf8 { "—" } else { "-" };
    let mut sorted = shown.to_vec();
    sorted.sort_unstable();
    let what = if sorted.len() == 1 {
        format!("page {} of {total}", sorted[0])
    } else {
        format!("pages {} of {total}", pages::compact(&sorted))
    };
    let last = *sorted.last()?;
    let next = if last < total {
        let to = (last + 3).min(total);
        if to == last + 1 {
            to.to_string()
        } else {
            format!("{}-{to}", last + 1)
        }
    } else {
        "all".into()
    };
    Some(format!("{what} {dash} use --pages {next} for more"))
}

fn note<W: Write>(out: &mut W, colors: bool, text: &str) -> io::Result<()> {
    if colors {
        writeln!(out, "\x1b[2m{text}\x1b[22m")
    } else {
        writeln!(out, "{text}")
    }
}

fn plural(n: u32, word: &str) -> String {
    if n == 1 {
        format!("1 {word}")
    } else {
        format!("{n} {word}s")
    }
}

/// A private copy of a stream: a 0600 file in a fresh 0700 directory, removed on drop.
struct TempCopy {
    dir: PathBuf,
    path: PathBuf,
    size: u64,
}

impl TempCopy {
    fn new<R: Read>(reader: &mut R) -> io::Result<TempCopy> {
        use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
        let base = std::env::temp_dir();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.subsec_nanos());
        let mut attempt = 0u32;
        let dir = loop {
            let dir = base.join(format!(
                "scat-pdf-{}-{nanos:x}-{attempt}",
                std::process::id()
            ));
            match std::fs::DirBuilder::new().mode(0o700).create(&dir) {
                Ok(()) => break dir,
                Err(err) if err.kind() == io::ErrorKind::AlreadyExists && attempt < 100 => {
                    attempt += 1;
                }
                Err(err) => return Err(err),
            }
        };
        let path = dir.join("input.pdf");
        let mut copy = TempCopy { dir, path, size: 0 };
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&copy.path)?;
        let size = io::copy(&mut reader.take(MAX_STREAM_BYTES + 1), &mut file)?;
        if size > MAX_STREAM_BYTES {
            return Err(io::Error::other(format!(
                "PDF stream larger than {} MiB, not shown",
                MAX_STREAM_BYTES >> 20
            )));
        }
        copy.size = size;
        Ok(copy)
    }
}

impl Drop for TempCopy {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
        let _ = std::fs::remove_dir(&self.dir);
    }
}

/// Whether a file goes to the PDF view with the `auto` renderer.
pub fn wants(mime: &str, head: &[u8]) -> bool {
    mime == "application/pdf" || head.starts_with(b"%PDF-")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn footer_suggests_the_next_pages() {
        assert_eq!(
            footer(&[1], 12, true).as_deref(),
            Some("page 1 of 12 — use --pages 2-4 for more")
        );
        assert_eq!(
            footer(&[1, 2, 3], 4, false).as_deref(),
            Some("pages 1-3 of 4 - use --pages 4 for more")
        );
        assert_eq!(
            footer(&[12], 12, true).as_deref(),
            Some("page 12 of 12 — use --pages all for more")
        );
        assert_eq!(footer(&[1], 1, true), None);
    }

    #[test]
    fn names_paper_sizes() {
        assert_eq!(page_size((595.0, 842.0)), "210 x 297 mm (A4)");
        assert_eq!(page_size((792.0, 612.0)), "279 x 216 mm (Letter landscape)");
        assert_eq!(page_size((100.0, 100.0)), "35 x 35 mm");
    }

    #[test]
    fn parses_pdfinfo_output() {
        let info = parse_pdfinfo(
            "Title:           BANDPLAN\nAuthor:          Theo\nPages:           34\n\
             Encrypted:       no\nPage size:       612 x 792 pts (letter)\nPDF version:     1.5\n",
        );
        assert_eq!(info.title.as_deref(), Some("BANDPLAN"));
        assert_eq!(info.pages, 34);
        assert_eq!(info.page_size, Some((612.0, 792.0)));
        assert_eq!(info.version.as_deref(), Some("1.5"));
        assert!(!info.encrypted);
    }

    #[test]
    fn groups_consecutive_pages() {
        assert_eq!(runs(&[1, 2, 3, 7, 9, 10]), vec![(1, 3), (7, 7), (9, 10)]);
    }
}
