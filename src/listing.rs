//! Directory listing: a grid like `ls`, or a long view with one entry per line.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::io::{self, Write};
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::path::Path;
use std::time::SystemTime;

use unicode_width::UnicodeWidthStr;

use crate::config::{ListStyle, NoArgs};
use crate::render::sanitize_str;
use crate::util::human_size;

/// What `scat` does when it is started without file arguments.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoArgsAction {
    ReadStdin,
    ListCurrentDir,
}

/// Lists the current directory only when a person is at the keyboard (stdin is a terminal),
/// scat is rendering, and the config does not ask for cat's behaviour.
pub fn no_args_action(stdin_tty: bool, rendering: bool, setting: NoArgs) -> NoArgsAction {
    if stdin_tty && rendering && setting == NoArgs::List {
        NoArgsAction::ListCurrentDir
    } else {
        NoArgsAction::ReadStdin
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Dir,
    Symlink { to_dir: bool, broken: bool },
    File { executable: bool },
    Socket,
    Pipe,
    BlockDevice,
    CharDevice,
}

#[derive(Debug, Clone)]
pub struct Entry {
    /// Name as stored on disk, lossily converted. Sanitized only when printed.
    pub name: String,
    pub kind: Kind,
    pub size: u64,
    pub modified: Option<SystemTime>,
    pub link_target: Option<String>,
}

impl Entry {
    fn is_dir_like(&self) -> bool {
        matches!(self.kind, Kind::Dir | Kind::Symlink { to_dir: true, .. })
    }

    fn hidden(&self) -> bool {
        self.name.starts_with('.')
    }
}

pub struct Options {
    pub all: bool,
    pub style: ListStyle,
    pub colors: bool,
    pub width: usize,
}

/// Reads the entries of `path`, sorted: directories first, then the rest, each group
/// case-insensitively with numbers compared by value.
pub fn read_entries(path: &Path, all: bool) -> io::Result<Vec<Entry>> {
    let mut entries = Vec::new();
    for item in std::fs::read_dir(path)? {
        let item = item?;
        let name = item.file_name().to_string_lossy().into_owned();
        if !all && name.starts_with('.') {
            continue;
        }
        let Ok(meta) = item.path().symlink_metadata() else {
            continue;
        };
        let ft = meta.file_type();
        let mut link_target = None;
        let kind = if ft.is_symlink() {
            let target = std::fs::metadata(item.path());
            link_target = std::fs::read_link(item.path())
                .ok()
                .map(|t| t.to_string_lossy().into_owned());
            Kind::Symlink {
                to_dir: target.as_ref().is_ok_and(|m| m.is_dir()),
                broken: target.is_err(),
            }
        } else if ft.is_dir() {
            Kind::Dir
        } else if ft.is_socket() {
            Kind::Socket
        } else if ft.is_fifo() {
            Kind::Pipe
        } else if ft.is_block_device() {
            Kind::BlockDevice
        } else if ft.is_char_device() {
            Kind::CharDevice
        } else {
            Kind::File {
                executable: meta.permissions().mode() & 0o111 != 0,
            }
        };
        entries.push(Entry {
            name,
            kind,
            size: meta.len(),
            modified: meta.modified().ok(),
            link_target,
        });
    }
    sort_entries(&mut entries);
    Ok(entries)
}

pub fn sort_entries(entries: &mut [Entry]) {
    entries.sort_by(|a, b| {
        b.is_dir_like()
            .cmp(&a.is_dir_like())
            .then_with(|| natural_cmp(&a.name, &b.name))
    });
}

/// Case-insensitive comparison where digit runs compare by value (`a2` < `a10`).
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (mut x, mut y) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (x.peek().copied(), y.peek().copied()) {
            (None, None) => return a.cmp(b),
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(c), Some(d)) if c.is_ascii_digit() && d.is_ascii_digit() => {
                let run = |it: &mut std::iter::Peekable<std::str::Chars>| {
                    let mut s = String::new();
                    while let Some(&ch) = it.peek() {
                        if !ch.is_ascii_digit() {
                            break;
                        }
                        s.push(ch);
                        it.next();
                    }
                    s
                };
                let (m, n) = (run(&mut x), run(&mut y));
                let (m, n) = (m.trim_start_matches('0'), n.trim_start_matches('0'));
                let ord = m.len().cmp(&n.len()).then_with(|| m.cmp(n));
                if ord != Ordering::Equal {
                    return ord;
                }
            }
            (Some(c), Some(d)) => {
                let ord = c.to_lowercase().cmp(d.to_lowercase());
                if ord != Ordering::Equal {
                    return ord;
                }
                x.next();
                y.next();
            }
        }
    }
}

/// Terminal width: the tty size, else `$COLUMNS`, else 80.
pub fn terminal_width() -> usize {
    if let Ok(ws) = rustix::termios::tcgetwinsize(std::io::stdout())
        && ws.ws_col > 0
    {
        return ws.ws_col as usize;
    }
    std::env::var("COLUMNS")
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .filter(|&w| w > 0)
        .unwrap_or(80)
}

/// A file name that is safe to print: control characters shown as ^X, invisible
/// formatting characters replaced, so the width stays predictable.
pub fn safe_name(name: &str) -> String {
    sanitize_str(name)
        .chars()
        .map(|c| {
            if c.is_control() || is_invisible(c) {
                '\u{fffd}'
            } else {
                c
            }
        })
        .collect()
}

/// Zero-width and invisible formatting characters. Bidi controls never get here:
/// `sanitize_str` already shows them as `<U+XXXX>`.
fn is_invisible(c: char) -> bool {
    matches!(c as u32, 0x200b..=0x200d | 0x2060..=0x2065 | 0x206a..=0x206f | 0xfeff | 0x180e)
}

/// Colours by file type, from `LS_COLORS` when set, otherwise defaults.
pub struct Palette {
    keys: HashMap<String, String>,
    suffixes: Vec<(String, String)>,
    from_env: bool,
}

impl Palette {
    pub fn from_env() -> Self {
        match std::env::var("LS_COLORS") {
            Ok(value) if !value.is_empty() => Self::parse(&value),
            _ => Self::parse(""),
        }
    }

    pub fn parse(spec: &str) -> Self {
        let mut keys = HashMap::new();
        let mut suffixes = Vec::new();
        for part in spec.split(':') {
            let Some((key, value)) = part.split_once('=') else {
                continue;
            };
            // Only SGR parameters: the value is printed inside an escape sequence.
            if !value.bytes().all(|b| b.is_ascii_digit() || b == b';') {
                continue;
            }
            if let Some(suffix) = key.strip_prefix('*') {
                suffixes.push((suffix.to_string(), value.to_string()));
            } else {
                keys.insert(key.to_string(), value.to_string());
            }
        }
        Self {
            keys,
            suffixes,
            from_env: !spec.is_empty(),
        }
    }

    fn key(&self, key: &str, default: &'static str) -> String {
        match self.keys.get(key) {
            Some(v) => v.clone(),
            None if self.from_env => String::new(),
            None => default.to_string(),
        }
    }

    /// SGR parameters for an entry; empty means no colour.
    pub fn style(&self, entry: &Entry) -> String {
        let style = match entry.kind {
            Kind::Dir => self.key("di", "1;34"),
            Kind::Symlink { broken: true, .. } => self.key("or", "31"),
            Kind::Symlink { .. } => self.key("ln", "36"),
            Kind::Socket => self.key("so", "35"),
            Kind::Pipe => self.key("pi", "33"),
            Kind::BlockDevice => self.key("bd", "33"),
            Kind::CharDevice => self.key("cd", "33"),
            Kind::File { executable } => {
                let by_suffix = self
                    .suffixes
                    .iter()
                    .rev()
                    .find(|(s, _)| entry.name.ends_with(s.as_str()))
                    .map(|(_, v)| v.clone());
                if let Some(v) = by_suffix {
                    v
                } else if executable {
                    self.key("ex", "1;32")
                } else {
                    self.key("fi", "")
                }
            }
        };
        if entry.hidden() && !self.from_env {
            return if style.is_empty() {
                "2".into()
            } else {
                format!("2;{style}")
            };
        }
        style
    }
}

fn paint(text: &str, style: &str, colors: bool) -> String {
    if colors && !style.is_empty() {
        format!("\x1b[{style}m{text}\x1b[0m")
    } else {
        text.to_string()
    }
}

/// Printable label (sanitized, `/` appended for directories) and its display width.
fn label(entry: &Entry) -> (String, usize) {
    let mut text = safe_name(&entry.name);
    if entry.kind == Kind::Dir {
        text.push('/');
    }
    let width = text.width();
    (text, width)
}

/// Number of columns and their widths for the widest layout that fits `width`.
fn grid_layout(widths: &[usize], width: usize) -> (usize, Vec<usize>) {
    const GAP: usize = 2;
    let count = widths.len();
    for cols in (1..=count.max(1)).rev() {
        let rows = count.div_ceil(cols);
        let real_cols = count.div_ceil(rows.max(1));
        if real_cols != cols {
            continue;
        }
        let col_widths: Vec<usize> = (0..cols)
            .map(|c| {
                widths[c * rows..((c + 1) * rows).min(count)]
                    .iter()
                    .copied()
                    .max()
                    .unwrap_or(0)
            })
            .collect();
        let total: usize = col_widths.iter().sum::<usize>() + GAP * (cols - 1);
        if total <= width || cols == 1 {
            return (rows, col_widths);
        }
    }
    (count, vec![widths.iter().copied().max().unwrap_or(0)])
}

pub fn render_grid<W: Write>(
    entries: &[Entry],
    opts: &Options,
    palette: &Palette,
    out: &mut W,
) -> io::Result<()> {
    if entries.is_empty() {
        return Ok(());
    }
    let labels: Vec<(String, usize)> = entries.iter().map(label).collect();
    let widths: Vec<usize> = labels.iter().map(|(_, w)| *w).collect();
    let (rows, col_widths) = grid_layout(&widths, opts.width);
    let count = entries.len();
    for row in 0..rows {
        let mut line = String::new();
        let cells: Vec<usize> = (0..col_widths.len())
            .map(|c| c * rows + row)
            .filter(|&i| i < count)
            .collect();
        for (pos, &i) in cells.iter().enumerate() {
            let (text, w) = &labels[i];
            line.push_str(&paint(text, &palette.style(&entries[i]), opts.colors));
            if pos + 1 < cells.len() {
                let pad = col_widths[pos] - w + 2;
                line.push_str(&" ".repeat(pad));
            }
        }
        line.push('\n');
        out.write_all(line.as_bytes())?;
    }
    Ok(())
}

fn type_char(kind: Kind) -> char {
    match kind {
        Kind::Dir => 'd',
        Kind::Symlink { .. } => 'l',
        Kind::File { .. } => '-',
        Kind::Socket => 's',
        Kind::Pipe => 'p',
        Kind::BlockDevice => 'b',
        Kind::CharDevice => 'c',
    }
}

/// "3 d ago" style age.
pub fn relative_age(secs: u64) -> String {
    const UNITS: [(u64, &str); 6] = [
        (365 * 86400, "y"),
        (30 * 86400, "mo"),
        (7 * 86400, "w"),
        (86400, "d"),
        (3600, "h"),
        (60, "min"),
    ];
    for (size, unit) in UNITS {
        if secs >= size {
            return format!("{} {unit} ago", secs / size);
        }
    }
    if secs < 5 {
        "now".into()
    } else {
        format!("{secs} s ago")
    }
}

pub fn render_long<W: Write>(
    entries: &[Entry],
    opts: &Options,
    palette: &Palette,
    now: SystemTime,
    out: &mut W,
) -> io::Result<()> {
    let sizes: Vec<String> = entries
        .iter()
        .map(|e| match e.kind {
            Kind::File { .. } => human_size(e.size),
            _ => "-".into(),
        })
        .collect();
    let ages: Vec<String> = entries
        .iter()
        .map(|e| {
            e.modified
                .map(|m| relative_age(now.duration_since(m).map_or(0, |d| d.as_secs())))
                .unwrap_or_else(|| "-".into())
        })
        .collect();
    let size_w = sizes.iter().map(|s| s.len()).max().unwrap_or(0);
    let age_w = ages.iter().map(|s| s.len()).max().unwrap_or(0);
    for (i, entry) in entries.iter().enumerate() {
        let (text, _) = label(entry);
        let mut name = paint(&text, &palette.style(entry), opts.colors);
        if let Some(target) = &entry.link_target {
            name.push_str(" -> ");
            name.push_str(&safe_name(target));
        }
        let meta = format!(
            "{} {:>size_w$}  {:>age_w$}",
            type_char(entry.kind),
            sizes[i],
            ages[i]
        );
        let meta = paint(&meta, "2", opts.colors);
        out.write_all(format!("{meta}  {name}\n").as_bytes())?;
    }
    Ok(())
}

/// Lists one directory.
pub fn list<W: Write>(path: &Path, opts: &Options, out: &mut W) -> io::Result<()> {
    let entries = read_entries(path, opts.all)?;
    let palette = Palette::from_env();
    match opts.style {
        ListStyle::Grid => render_grid(&entries, opts, &palette, out),
        ListStyle::Long => render_long(&entries, opts, &palette, SystemTime::now(), out),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, kind: Kind) -> Entry {
        Entry {
            name: name.into(),
            kind,
            size: 0,
            modified: None,
            link_target: None,
        }
    }

    fn file(name: &str) -> Entry {
        entry(name, Kind::File { executable: false })
    }

    #[test]
    fn no_args_decision() {
        use NoArgsAction::*;
        assert_eq!(no_args_action(true, true, NoArgs::List), ListCurrentDir);
        assert_eq!(no_args_action(false, true, NoArgs::List), ReadStdin);
        assert_eq!(no_args_action(true, false, NoArgs::List), ReadStdin);
        assert_eq!(no_args_action(true, true, NoArgs::Stdin), ReadStdin);
    }

    #[test]
    fn dirs_first_natural_case_insensitive() {
        let mut v = vec![
            file("b10"),
            file("B2"),
            entry("zdir", Kind::Dir),
            file("a"),
            entry("Adir", Kind::Dir),
        ];
        sort_entries(&mut v);
        let names: Vec<&str> = v.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["Adir", "zdir", "a", "B2", "b10"]);
    }

    #[test]
    fn grid_fills_columns_top_to_bottom() {
        let entries: Vec<Entry> = ["a", "b", "c", "d", "e"].iter().map(|n| file(n)).collect();
        let opts = Options {
            all: false,
            style: ListStyle::Grid,
            colors: false,
            width: 7,
        };
        let mut out = Vec::new();
        render_grid(&entries, &opts, &Palette::parse(""), &mut out).unwrap();
        assert_eq!(String::from_utf8(out).unwrap(), "a  c  e\nb  d\n");
    }

    #[test]
    fn grid_uses_display_width() {
        let entries = vec![file("日本"), file("x"), file("y")];
        let opts = Options {
            all: false,
            style: ListStyle::Grid,
            colors: false,
            width: 6,
        };
        let mut out = Vec::new();
        render_grid(&entries, &opts, &Palette::parse(""), &mut out).unwrap();
        // two rows of width 4+2+1 do not fit in 6; one column
        assert_eq!(String::from_utf8(out).unwrap(), "日本\nx\ny\n");
    }

    #[test]
    fn names_are_sanitized() {
        let name = safe_name("a\x1b[31m\nb\u{202e}c\u{9b}");
        assert!(!name.contains('\x1b'));
        assert!(!name.contains('\n'));
        assert!(!name.contains('\u{202e}'));
    }

    #[test]
    fn ls_colors_parsing() {
        let p = Palette::parse("di=01;35:*.rs=33:ex=0;1\x1b:ln=bad");
        assert_eq!(p.style(&entry("d", Kind::Dir)), "01;35");
        assert_eq!(p.style(&file("main.rs")), "33");
        assert_eq!(
            p.style(&entry("x", Kind::File { executable: true })),
            String::new()
        );
        assert_eq!(p.style(&file("plain")), "");
    }

    #[test]
    fn default_palette_dims_hidden() {
        let p = Palette::parse("");
        assert_eq!(p.style(&file(".env")), "2");
        assert_eq!(p.style(&file("env")), "");
        assert_eq!(p.style(&entry("d", Kind::Dir)), "1;34");
    }

    #[test]
    fn ages() {
        assert_eq!(relative_age(0), "now");
        assert_eq!(relative_age(30), "30 s ago");
        assert_eq!(relative_age(3 * 86400 + 5), "3 d ago");
    }
}
