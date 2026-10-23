//! Archive listing: a summary and a file tree for zip, tar (plain or compressed), deb
//! and 7z archives. Nothing is extracted; sizes come from the archive's own metadata.
//!
//! Archives are untrusted. Names are only displayed (sanitized), the number of entries
//! scanned and the bytes decompressed are capped, and tar archives are read as a stream.

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::fmt::Display;
use std::fs::File;
use std::io::{self, Cursor, Read, Seek, Write};
use std::path::Path;
use std::rc::Rc;

use unicode_width::UnicodeWidthStr;

use crate::decompress;
use crate::kv;
use crate::listing::{self, Palette, safe_name};
use crate::util::human_size;

/// Entries scanned before giving up on counting (the tree shows far fewer).
const SCAN_LIMIT: u64 = 1_000_000;
/// Largest archive read into memory from a pipe, for formats that need seeking.
const BUFFER_LIMIT: u64 = 256 * 1024 * 1024;
/// Longest entry name kept, in bytes.
const NAME_LIMIT: usize = 4096;
/// Deepest nesting drawn; deeper components are folded into one name.
const DEPTH_LIMIT: usize = 64;
/// Longest symlink target read from a zip entry.
const LINK_LIMIT: u64 = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Zip,
    Tar,
    Tarball(decompress::Format),
    Deb,
    SevenZ,
}

/// Which archive view applies to a file, judged by magic bytes and name. `head` is the
/// start of the raw file.
pub fn detect(head: &[u8], file_name: Option<&str>) -> Option<Format> {
    if head.starts_with(b"PK\x03\x04") || head.starts_with(b"PK\x05\x06") {
        return Some(Format::Zip);
    }
    if head.starts_with(b"7z\xbc\xaf\x27\x1c") {
        return Some(Format::SevenZ);
    }
    if head.starts_with(b"!<arch>\n") {
        let named_deb = file_name.is_some_and(|n| n.to_ascii_lowercase().ends_with(".deb"));
        return (named_deb || head[8..].starts_with(b"debian-binary")).then_some(Format::Deb);
    }
    if let Some(format) = decompress::detect(head) {
        return decompress::is_tarball(file_name, head, format).then_some(Format::Tarball(format));
    }
    let named_tar = file_name.is_some_and(|n| n.to_ascii_lowercase().ends_with(".tar"));
    let ustar = head.len() >= 262 && &head[257..262] == b"ustar";
    (ustar || (named_tar && head.len() >= 512)).then_some(Format::Tar)
}

pub struct Options {
    /// Entries drawn per section; 0 draws all.
    pub max_entries: usize,
    pub colors: bool,
    pub utf8: bool,
    pub width: usize,
    /// Decompressed bytes read from a compressed tarball before the scan stops.
    pub max_scan_bytes: u64,
}

/// Where the archive comes from: its bytes, a path to reopen for seeking, its size.
pub struct Source<'a> {
    pub reader: Box<dyn Read>,
    pub path: Option<&'a Path>,
    pub size: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Date {
    year: i32,
    month: u32,
    day: u32,
}

impl Date {
    fn from_unix(secs: i64) -> Self {
        // Days since 1970-01-01 to a civil date (Howard Hinnant's algorithm).
        let z = secs.div_euclid(86_400) + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z.rem_euclid(146_097);
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
        let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
        let year = (yoe + era * 400) as i32 + i32::from(month <= 2);
        Self { year, month, day }
    }
}

impl Display for Date {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ItemKind {
    Dir,
    File { executable: bool },
    Symlink(Option<String>),
    Hardlink(String),
}

#[derive(Debug, Clone)]
struct Item {
    path: String,
    kind: ItemKind,
    size: u64,
    date: Option<Date>,
}

/// Entries of one tar stream (or the whole archive), as far as they are kept.
#[derive(Default)]
struct Section {
    title: Option<String>,
    items: Vec<Item>,
    entries: u64,
    bytes: u64,
    /// Scanning stopped early, so the counts are lower bounds.
    limited: bool,
    /// Why a sequential read ended before the end of the archive.
    error: Option<String>,
    keep: usize,
}

impl Section {
    fn new(keep: usize) -> Self {
        Self {
            keep: if keep == 0 { usize::MAX } else { keep },
            ..Self::default()
        }
    }

    fn full(&self) -> bool {
        self.entries >= SCAN_LIMIT
    }

    fn add(&mut self, mut item: Item) {
        if item.path.len() > NAME_LIMIT {
            let mut end = NAME_LIMIT;
            while !item.path.is_char_boundary(end) {
                end -= 1;
            }
            item.path.truncate(end);
        }
        // The archive root ("./") is not an entry.
        if item
            .path
            .split('/')
            .all(|part| part.is_empty() || part == ".")
        {
            return;
        }
        self.entries += 1;
        if matches!(item.kind, ItemKind::File { .. }) {
            self.bytes = self.bytes.saturating_add(item.size);
        }
        if self.items.len() < self.keep {
            self.items.push(item);
        }
    }
}

struct Listing {
    label: String,
    /// Size of the archive file when it is compressed (or a container).
    packed: Option<u64>,
    sections: Vec<Section>,
}

fn bad(format: &str, err: impl Display) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("not a valid {format} archive: {err}"),
    )
}

trait ReadSeek: Read + Seek {}
impl<T: Read + Seek> ReadSeek for T {}

/// A seekable view of the archive: the file reopened, or a pipe buffered in memory.
fn seekable(source: Source) -> io::Result<(Box<dyn ReadSeek>, Option<u64>)> {
    if let Some(path) = source.path {
        let file = File::open(path)?;
        let size = file.metadata()?.len();
        return Ok((Box::new(file), Some(size)));
    }
    let mut data = Vec::new();
    source
        .reader
        .take(BUFFER_LIMIT + 1)
        .read_to_end(&mut data)?;
    if data.len() as u64 > BUFFER_LIMIT {
        return Err(io::Error::other(format!(
            "too large to list from a pipe (over {})",
            human_size(BUFFER_LIMIT)
        )));
    }
    let size = data.len() as u64;
    Ok((Box::new(Cursor::new(data)), Some(size)))
}

pub fn render<W: Write>(
    out: &mut W,
    format: Format,
    name: &str,
    source: Source,
    opts: &Options,
) -> io::Result<()> {
    let listing = scan(format, name, source, opts)?;
    write_listing(out, name, &listing, opts)
}

fn scan(format: Format, name: &str, source: Source, opts: &Options) -> io::Result<Listing> {
    let file_size = source.size;
    match format {
        Format::Zip => {
            let (reader, size) = seekable(source)?;
            let mut section = Section::new(opts.max_entries);
            scan_zip(reader, &mut section)?;
            Ok(Listing {
                label: zip_label(name),
                packed: size,
                sections: vec![section],
            })
        }
        Format::SevenZ => {
            let (reader, size) = seekable(source)?;
            let mut section = Section::new(opts.max_entries);
            scan_sevenz(reader, &mut section)?;
            Ok(Listing {
                label: "7z archive".into(),
                packed: size,
                sections: vec![section],
            })
        }
        Format::Tar => {
            let mut section = Section::new(opts.max_entries);
            match source.path {
                Some(path) => {
                    let mut archive = tar::Archive::new(File::open(path)?);
                    scan_tar(archive.entries_with_seek(), &mut section, "tar")?;
                }
                None => {
                    let mut archive = tar::Archive::new(source.reader);
                    scan_tar(archive.entries(), &mut section, "tar")?;
                }
            }
            Ok(Listing {
                label: "tar archive".into(),
                packed: None,
                sections: vec![section],
            })
        }
        Format::Tarball(compression) => {
            let mut section = Section::new(opts.max_entries);
            scan_compressed_tar(compression, source.reader, opts, &mut section)?;
            Ok(Listing {
                label: format!("tar archive ({})", compression_name(compression)),
                packed: file_size,
                sections: vec![section],
            })
        }
        Format::Deb => {
            let sections = scan_deb(source.reader, opts)?;
            Ok(Listing {
                label: "Debian package".into(),
                packed: file_size,
                sections,
            })
        }
    }
}

fn compression_name(format: decompress::Format) -> &'static str {
    match format {
        decompress::Format::Gzip => "gzip",
        decompress::Format::Xz => "xz",
        decompress::Format::Zstd => "zstd",
        decompress::Format::Bzip2 => "bzip2",
    }
}

fn zip_label(name: &str) -> String {
    let lower = name.to_ascii_lowercase();
    let kind = [
        (".jar", "Java archive"),
        (".war", "Java web archive"),
        (".whl", "Python wheel"),
        (".apk", "Android package"),
        (".epub", "EPUB book"),
        (".xpi", "Browser extension"),
    ]
    .iter()
    .find(|(ext, _)| lower.ends_with(ext));
    match kind {
        Some((_, label)) => format!("{label} (zip)"),
        None => "zip archive".into(),
    }
}

fn lossy(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn scan_zip<R: Read + Seek>(reader: R, section: &mut Section) -> io::Result<()> {
    let mut archive = zip::ZipArchive::new(reader).map_err(|e| bad("zip", e))?;
    for index in 0..archive.len() {
        if section.full() {
            section.limited = true;
            break;
        }
        let mut file = archive.by_index_raw(index).map_err(|e| bad("zip", e))?;
        let path = match file.name() {
            Ok(name) => name.into_owned(),
            Err(_) => lossy(file.name_raw()),
        };
        let mode = file.unix_mode().unwrap_or(0);
        let kind = if file.is_dir() {
            ItemKind::Dir
        } else if file.is_symlink() {
            let mut target = Vec::new();
            // Symlinks are stored uncompressed; anything else is shown without target.
            if file.compression() == zip::CompressionMethod::Stored
                && file.size() <= LINK_LIMIT
                && (&mut file)
                    .take(LINK_LIMIT)
                    .read_to_end(&mut target)
                    .is_ok()
                && !target.is_empty()
            {
                ItemKind::Symlink(Some(lossy(&target)))
            } else {
                ItemKind::Symlink(None)
            }
        } else {
            ItemKind::File {
                executable: mode & 0o111 != 0,
            }
        };
        // 1980-01-01 is the DOS "no date" value that most tools write.
        let date = file
            .last_modified()
            .filter(|d| d.is_valid() && !(d.year() <= 1980 && d.month() == 1 && d.day() == 1))
            .map(|d| Date {
                year: i32::from(d.year()),
                month: u32::from(d.month()),
                day: u32::from(d.day()),
            });
        let size = file.size();
        section.add(Item {
            path,
            kind,
            size,
            date,
        });
    }
    Ok(())
}

fn scan_sevenz(mut reader: Box<dyn ReadSeek>, section: &mut Section) -> io::Result<()> {
    let archive = sevenz_rust2::Archive::read(&mut reader, &sevenz_rust2::Password::empty())
        .map_err(|e| bad("7z", e))?;
    for entry in &archive.files {
        if section.full() {
            section.limited = true;
            break;
        }
        if entry.is_anti_item() {
            continue;
        }
        // Unix mode lives in the high 16 bits when bit 15 of the attributes is set.
        let attributes = entry.windows_attributes();
        let mode = if attributes & 0x8000 != 0 {
            attributes >> 16
        } else {
            0
        };
        let kind = if entry.is_directory() {
            ItemKind::Dir
        } else if mode & 0o170_000 == 0o120_000 {
            ItemKind::Symlink(None)
        } else {
            ItemKind::File {
                executable: mode & 0o111 != 0,
            }
        };
        let date = entry.has_last_modified_date.then(|| {
            let time = std::time::SystemTime::from(entry.last_modified_date());
            match time.duration_since(std::time::UNIX_EPOCH) {
                Ok(d) => Date::from_unix(d.as_secs() as i64),
                Err(e) => Date::from_unix(-(e.duration().as_secs() as i64)),
            }
        });
        section.add(Item {
            path: entry.name().to_string(),
            kind,
            size: entry.size(),
            date: date.filter(|d| d.year > 1601),
        });
    }
    Ok(())
}

/// Reads the headers of a tar stream, skipping the data. A read error after the first
/// entry keeps what was seen and is shown under the listing; before it, the error is
/// returned.
fn scan_tar<'a, R: Read + 'a>(
    entries: io::Result<tar::Entries<'a, R>>,
    section: &mut Section,
    format: &str,
) -> io::Result<()> {
    let result = (|| -> io::Result<()> {
        for entry in entries? {
            if section.full() {
                section.limited = true;
                break;
            }
            let entry = entry?;
            let header = entry.header();
            let kind = header.entry_type();
            if kind.is_pax_global_extensions() {
                continue;
            }
            let path = lossy(&entry.path_bytes());
            let mode = header.mode().unwrap_or(0);
            let target = || entry.link_name_bytes().map(|t| lossy(&t));
            let kind = if kind.is_dir() {
                ItemKind::Dir
            } else if kind.is_symlink() {
                ItemKind::Symlink(target())
            } else if kind.is_hard_link() {
                ItemKind::Hardlink(target().unwrap_or_default())
            } else {
                ItemKind::File {
                    executable: mode & 0o111 != 0,
                }
            };
            let date = header
                .mtime()
                .ok()
                .filter(|&t| t > 0)
                .map(|t| Date::from_unix(t.min(i64::MAX as u64) as i64));
            let size = entry.size();
            section.add(Item {
                path,
                kind,
                size,
                date,
            });
        }
        Ok(())
    })();
    match result {
        Ok(()) => Ok(()),
        Err(err) if section.entries == 0 => Err(bad(format, err)),
        Err(err) => {
            section.error = Some(err.to_string());
            Ok(())
        }
    }
}

fn scan_compressed_tar(
    compression: decompress::Format,
    reader: Box<dyn Read>,
    opts: &Options,
    section: &mut Section,
) -> io::Result<()> {
    let (stream, hit) =
        decompress::open(compression, reader, opts.max_scan_bytes).map_err(|e| bad("tar", e))?;
    let mut archive = tar::Archive::new(stream);
    scan_tar(archive.entries(), section, "tar")?;
    if hit.get() {
        section.limited = true;
        // The cut-off stream ends mid-entry; that is not a corrupt archive.
        section.error = None;
    }
    Ok(())
}

/// A bounded window of the shared `ar` stream, so tar readers can own it.
struct Member {
    inner: Rc<RefCell<Box<dyn Read>>>,
    left: Rc<Cell<u64>>,
}

impl Read for Member {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let left = self.left.get();
        if left == 0 || buf.is_empty() {
            return Ok(0);
        }
        let max = buf.len().min(usize::try_from(left).unwrap_or(usize::MAX));
        let n = self.inner.borrow_mut().read(&mut buf[..max])?;
        self.left.set(left - n as u64);
        Ok(n)
    }
}

/// Lists the tar members of a `.deb` (an `ar` archive): control and data.
fn scan_deb(reader: Box<dyn Read>, opts: &Options) -> io::Result<Vec<Section>> {
    let shared = Rc::new(RefCell::new(reader));
    let mut magic = [0u8; 8];
    shared
        .borrow_mut()
        .read_exact(&mut magic)
        .map_err(|e| bad("deb", e))?;
    if &magic != b"!<arch>\n" {
        return Err(bad("deb", "missing ar header"));
    }
    let mut sections: Vec<Section> = Vec::new();
    let mut kept = 0usize;
    loop {
        let mut header = [0u8; 60];
        let mut filled = 0;
        while filled < header.len() {
            let n = shared
                .borrow_mut()
                .read(&mut header[filled..])
                .map_err(|e| bad("deb", e))?;
            if n == 0 {
                break;
            }
            filled += n;
        }
        if filled == 0 {
            break;
        }
        if filled < header.len() || &header[58..60] != b"`\n" {
            return Err(bad("deb", "truncated or damaged ar member header"));
        }
        let name = lossy(&header[..16]);
        let name = name.trim_end().trim_end_matches('/').to_string();
        let size: u64 = lossy(&header[48..58])
            .trim()
            .parse()
            .map_err(|_| bad("deb", "bad member size"))?;
        let left = Rc::new(Cell::new(size));
        let member = || Member {
            inner: shared.clone(),
            left: left.clone(),
        };
        let wanted = name.starts_with("control.tar") || name.starts_with("data.tar");
        let mut stop = false;
        if wanted {
            let keep = if opts.max_entries == 0 {
                0
            } else {
                opts.max_entries.saturating_sub(kept).max(1)
            };
            let mut section = Section::new(keep);
            section.title = Some(name.clone());
            let mut prefix = Vec::new();
            member().take(6).read_to_end(&mut prefix)?;
            let source: Box<dyn Read> = Box::new(Cursor::new(prefix.clone()).chain(member()));
            match decompress::detect(&prefix) {
                Some(compression) => {
                    scan_compressed_tar(compression, source, opts, &mut section)?;
                }
                None => {
                    let mut archive = tar::Archive::new(source);
                    scan_tar(archive.entries(), &mut section, "deb")?;
                }
            }
            kept += section.items.len();
            stop = section.limited;
            sections.push(section);
        }
        if stop {
            break;
        }
        io::copy(&mut member(), &mut io::sink())?;
        if size % 2 == 1 {
            let mut pad = [0u8; 1];
            let _ = shared.borrow_mut().read(&mut pad)?;
        }
    }
    if sections.is_empty() {
        return Err(bad("deb", "no control or data archive found"));
    }
    Ok(sections)
}

// --- rendering ---

#[derive(Default)]
struct Node {
    item: Option<Item>,
    children: BTreeMap<String, Node>,
}

impl Node {
    fn is_dir(&self) -> bool {
        !self.children.is_empty() || self.item.as_ref().is_none_or(|i| i.kind == ItemKind::Dir)
    }
}

/// Path components for display: empty and `.` parts dropped, `..` kept, an absolute path
/// keeps a leading `/`, nesting beyond [`DEPTH_LIMIT`] folded into the last component.
fn components(path: &str) -> Vec<String> {
    let mut parts: Vec<String> = Vec::new();
    if path.starts_with('/') {
        parts.push("/".into());
    }
    for part in path.split('/') {
        if part.is_empty() || part == "." {
            continue;
        }
        if parts.len() >= DEPTH_LIMIT {
            let last = parts.last_mut().expect("depth limit is above zero");
            last.push('/');
            last.push_str(part);
        } else {
            parts.push(part.to_string());
        }
    }
    parts
}

fn build_tree(items: &[Item]) -> Node {
    let mut root = Node::default();
    for item in items {
        let parts = components(&item.path);
        if parts.is_empty() {
            continue;
        }
        let mut node = &mut root;
        for part in &parts {
            node = node.children.entry(part.clone()).or_default();
        }
        node.item = Some(item.clone());
    }
    root
}

struct Row {
    prefix: String,
    name: String,
    style: String,
    suffix: String,
    size: Option<u64>,
    date: Option<Date>,
}

struct Glyphs {
    branch: &'static str,
    last: &'static str,
    pipe: &'static str,
    blank: &'static str,
    ellipsis: &'static str,
}

impl Glyphs {
    fn new(utf8: bool) -> Self {
        if utf8 {
            Self {
                branch: "├── ",
                last: "└── ",
                pipe: "│   ",
                blank: "    ",
                ellipsis: "…",
            }
        } else {
            Self {
                branch: "|-- ",
                last: "`-- ",
                pipe: "|   ",
                blank: "    ",
                ellipsis: "...",
            }
        }
    }
}

fn collect_rows(
    node: &Node,
    prefix: &str,
    glyphs: &Glyphs,
    palette: &Palette,
    rows: &mut Vec<Row>,
) {
    let mut children: Vec<(&String, &Node)> = node.children.iter().collect();
    children.sort_by(|a, b| {
        b.1.is_dir()
            .cmp(&a.1.is_dir())
            .then_with(|| listing::natural_cmp(a.0, b.0))
    });
    let count = children.len();
    for (i, (name, child)) in children.into_iter().enumerate() {
        let last = i + 1 == count;
        let item = child.item.as_ref();
        let kind = match item.map(|i| &i.kind) {
            _ if child.is_dir() => listing::Kind::Dir,
            Some(ItemKind::Symlink(_)) => listing::Kind::Symlink {
                to_dir: false,
                broken: false,
            },
            Some(ItemKind::File { executable }) => listing::Kind::File {
                executable: *executable,
            },
            _ => listing::Kind::File { executable: false },
        };
        let style = palette.style(&listing::Entry {
            name: name.clone(),
            kind,
            size: 0,
            modified: None,
            link_target: None,
        });
        let mut suffix = String::new();
        if child.is_dir() {
            if !name.ends_with('/') {
                suffix.push('/');
            }
        } else {
            match item.map(|i| &i.kind) {
                Some(ItemKind::File { executable: true }) => suffix.push('*'),
                Some(ItemKind::Symlink(Some(target))) => {
                    suffix.push_str(" -> ");
                    suffix.push_str(&safe_name(target));
                }
                Some(ItemKind::Symlink(None)) => suffix.push('@'),
                Some(ItemKind::Hardlink(target)) => {
                    suffix.push_str(" => ");
                    suffix.push_str(&safe_name(target));
                }
                _ => {}
            }
        }
        rows.push(Row {
            prefix: format!("{prefix}{}", if last { glyphs.last } else { glyphs.branch }),
            name: safe_name(name),
            style,
            suffix,
            size: item
                .filter(|i| !child.is_dir() && matches!(i.kind, ItemKind::File { .. }))
                .map(|i| i.size),
            date: item.and_then(|i| i.date),
        });
        if !child.children.is_empty() {
            let next = format!("{prefix}{}", if last { glyphs.blank } else { glyphs.pipe });
            collect_rows(child, &next, glyphs, palette, rows);
        }
    }
}

fn paint(text: &str, style: &str, colors: bool) -> String {
    if colors && !style.is_empty() && !text.is_empty() {
        format!("\x1b[{style}m{text}\x1b[0m")
    } else {
        text.to_string()
    }
}

fn group(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

fn write_listing<W: Write>(
    out: &mut W,
    name: &str,
    listing: &Listing,
    opts: &Options,
) -> io::Result<()> {
    let entries: u64 = listing.sections.iter().map(|s| s.entries).sum();
    let bytes: u64 = listing
        .sections
        .iter()
        .fold(0, |acc, s| acc.saturating_add(s.bytes));
    let limited = listing.sections.iter().any(|s| s.limited);
    let more = if limited { "+" } else { "" };
    let mut rows = vec![
        ("Type", listing.label.clone()),
        ("Entries", format!("{}{more}", group(entries))),
        ("Size", format!("{}{more} uncompressed", human_size(bytes))),
    ];
    if let Some(packed) = listing.packed {
        let ratio = if bytes > 0 && !limited {
            format!(" ({}%)", (packed as f64 / bytes as f64 * 100.0).round())
        } else {
            String::new()
        };
        rows.push(("Compressed", format!("{}{ratio}", human_size(packed))));
    }
    kv::write_table(out, name, &rows, opts.colors)?;

    let glyphs = Glyphs::new(opts.utf8);
    let palette = Palette::from_env();
    let (bold, dim, reset) = if opts.colors {
        ("\x1b[1m", "\x1b[2m", "\x1b[0m")
    } else {
        ("", "", "")
    };
    let titled = listing.sections.len() > 1;
    for section in &listing.sections {
        writeln!(out)?;
        if titled && let Some(title) = &section.title {
            writeln!(
                out,
                "{bold}{}{reset} ({} {})",
                safe_name(title),
                group(section.entries),
                if section.entries == 1 {
                    "entry"
                } else {
                    "entries"
                }
            )?;
        }
        let tree = build_tree(&section.items);
        let mut lines = Vec::new();
        collect_rows(&tree, "", &glyphs, &palette, &mut lines);

        let size_width = lines
            .iter()
            .filter_map(|r| r.size)
            .map(|s| human_size(s).len())
            .max()
            .unwrap_or(0);
        let has_dates = lines.iter().any(|r| r.date.is_some());
        let name_width = |r: &Row| r.prefix.width() + r.name.width() + r.suffix.width();
        let longest = lines.iter().map(name_width).max().unwrap_or(0);
        let extra =
            if size_width > 0 { size_width + 2 } else { 0 } + if has_dates { 12 } else { 0 };
        let column = longest.min(opts.width.saturating_sub(extra).max(24));
        for row in &lines {
            let mut line = format!(
                "{}{}{}",
                row.prefix,
                paint(&row.name, &row.style, opts.colors),
                row.suffix
            );
            let used = name_width(row);
            let mut tail = String::new();
            if size_width > 0 {
                if let Some(size) = row.size {
                    tail.push_str(&format!("  {:>size_width$}", human_size(size)));
                } else {
                    tail.push_str(&" ".repeat(size_width + 2));
                }
            }
            if let Some(date) = row.date {
                tail.push_str(&format!("  {dim}{date}{reset}"));
            }
            if !tail.trim().is_empty() {
                line.push_str(&" ".repeat(column.saturating_sub(used)));
                line.push_str(&tail);
            }
            writeln!(out, "{}", line.trim_end())?;
        }
        let hidden = section.entries.saturating_sub(section.items.len() as u64);
        if hidden > 0 || section.limited {
            let plus = if section.limited { "+" } else { "" };
            writeln!(
                out,
                "{dim}{} {}{plus} more entries{reset}",
                glyphs.ellipsis,
                group(hidden)
            )?;
        }
        if section.limited {
            writeln!(
                out,
                "{dim}{} listing stopped at the scan limit{reset}",
                glyphs.ellipsis
            )?;
        }
        if let Some(error) = &section.error {
            writeln!(
                out,
                "{dim}{} listing stopped early: {}{reset}",
                glyphs.ellipsis,
                crate::render::sanitize_str(error)
            )?;
        }
        if section.entries == 0 {
            writeln!(out, "{dim}(empty){reset}")?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unix_dates() {
        assert_eq!(Date::from_unix(0).to_string(), "1970-01-01");
        assert_eq!(Date::from_unix(1_700_000_000).to_string(), "2023-11-14");
        assert_eq!(Date::from_unix(951_782_400).to_string(), "2000-02-29");
    }

    #[test]
    fn detects_formats() {
        assert_eq!(detect(b"PK\x03\x04rest", None), Some(Format::Zip));
        assert_eq!(detect(b"7z\xbc\xaf\x27\x1c\0", None), Some(Format::SevenZ));
        assert_eq!(
            detect(b"!<arch>\ndebian-binary   ", None),
            Some(Format::Deb)
        );
        assert_eq!(detect(b"!<arch>\nfoo.o/", Some("libfoo.a")), None);
        assert_eq!(detect(b"plain text", Some("a.txt")), None);
    }

    #[test]
    fn path_components() {
        assert_eq!(components("./a//b/"), ["a", "b"]);
        assert_eq!(components("/etc/../x"), ["/", "etc", "..", "x"]);
        let deep = vec!["d"; 70].join("/");
        let parts = components(&deep);
        assert_eq!(parts.len(), DEPTH_LIMIT);
        assert_eq!(parts[DEPTH_LIMIT - 1], "d/d/d/d/d/d/d");
    }

    #[test]
    fn groups_digits() {
        assert_eq!(group(0), "0");
        assert_eq!(group(1234567), "1,234,567");
    }
}
