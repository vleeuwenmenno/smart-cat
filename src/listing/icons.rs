//! Icons for tiles without a thumbnail: a folder for directories, a badge with the
//! extension for files. Drawn with text only, so they work with every image method.

use unicode_width::UnicodeWidthStr;

use super::{Entry, Kind};
use crate::config::Icons;

/// One line of an icon: text (with SGR colours when enabled) and its display width.
pub type Line = (String, usize);

/// Broad file category; picks the badge colour and the Nerd Font glyph.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    Folder,
    Code,
    Config,
    Document,
    Archive,
    Image,
    Video,
    Audio,
    Executable,
    Special,
    Other,
}

impl Category {
    /// ANSI colour number (0-7) of the badge.
    fn color(self) -> u8 {
        match self {
            Category::Folder => 4,
            Category::Code => 2,
            Category::Config => 3,
            Category::Document => 4,
            Category::Archive => 1,
            Category::Image | Category::Video => 5,
            Category::Audio => 6,
            Category::Executable => 2,
            Category::Special => 3,
            Category::Other => 7,
        }
    }
}

fn extension(name: &str) -> Option<&str> {
    let (stem, ext) = name.rsplit_once('.')?;
    (!stem.is_empty() && !ext.is_empty()).then_some(ext)
}

pub fn category(entry: &Entry) -> Category {
    match entry.kind {
        Kind::Dir | Kind::Symlink { to_dir: true, .. } => return Category::Folder,
        Kind::Socket | Kind::Pipe | Kind::BlockDevice | Kind::CharDevice => {
            return Category::Special;
        }
        _ => {}
    }
    let ext = extension(&entry.name).map(str::to_ascii_lowercase);
    match ext.as_deref() {
        Some(
            "rs" | "py" | "js" | "mjs" | "ts" | "tsx" | "jsx" | "go" | "c" | "h" | "cc" | "cpp"
            | "hpp" | "java" | "kt" | "rb" | "php" | "sh" | "bash" | "zsh" | "fish" | "lua"
            | "swift" | "zig" | "hs" | "ml" | "cs" | "dart" | "scala" | "ex" | "exs" | "pl" | "r"
            | "sql" | "html" | "css" | "scss" | "vue" | "svelte" | "nix",
        ) => Category::Code,
        Some(
            "toml" | "yaml" | "yml" | "json" | "jsonc" | "ini" | "conf" | "cfg" | "xml" | "csv"
            | "tsv" | "env" | "lock" | "properties",
        ) => Category::Config,
        Some(
            "md" | "markdown" | "txt" | "rst" | "org" | "adoc" | "tex" | "pdf" | "doc" | "docx"
            | "odt" | "rtf" | "epub" | "log",
        ) => Category::Document,
        Some(
            "zip" | "tar" | "gz" | "tgz" | "xz" | "zst" | "bz2" | "7z" | "rar" | "deb" | "rpm"
            | "iso" | "flatpak" | "apk" | "ipa" | "jar" | "dmg",
        ) => Category::Archive,
        Some(
            "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "ico" | "tif" | "tiff" | "svg"
            | "qoi" | "tga" | "heic" | "avif" | "psd" | "xcf",
        ) => Category::Image,
        Some(
            "mp4" | "m4v" | "mkv" | "webm" | "mov" | "avi" | "wmv" | "flv" | "mpg" | "mpeg" | "ogv"
            | "3gp",
        ) => Category::Video,
        Some(
            "mp3" | "flac" | "ogg" | "oga" | "opus" | "m4a" | "aac" | "wav" | "aiff" | "wv" | "ape"
            | "mid" | "midi",
        ) => Category::Audio,
        _ if matches!(entry.kind, Kind::File { executable: true }) => Category::Executable,
        _ => Category::Other,
    }
}

/// Text on a file badge: the extension in capitals (at most `max` characters), or a
/// word for files without one.
pub fn badge_label(entry: &Entry, max: usize) -> String {
    let text = match entry.kind {
        Kind::Socket => "SOCK".to_string(),
        Kind::Pipe => "FIFO".to_string(),
        Kind::BlockDevice | Kind::CharDevice => "DEV".to_string(),
        Kind::Symlink { broken: true, .. } => "LINK".to_string(),
        _ => match extension(&entry.name) {
            Some(ext) => ext
                .chars()
                .filter(|c| c.is_ascii_alphanumeric())
                .map(|c| c.to_ascii_uppercase())
                .collect(),
            None if matches!(entry.kind, Kind::File { executable: true }) => "EXE".into(),
            None => String::new(),
        },
    };
    text.chars().take(max).collect()
}

/// Nerd Font glyph for an entry.
pub fn nerd_glyph(entry: &Entry) -> char {
    let ext = extension(&entry.name).map(str::to_ascii_lowercase);
    match (category(entry), ext.as_deref()) {
        (Category::Folder, _) => '\u{f07b}',
        (_, Some("rs")) => '\u{e7a8}',
        (_, Some("py")) => '\u{e73c}',
        (_, Some("js" | "mjs")) => '\u{e74e}',
        (_, Some("ts" | "tsx")) => '\u{e628}',
        (_, Some("go")) => '\u{e627}',
        (_, Some("c" | "h")) => '\u{e61e}',
        (_, Some("cpp" | "cc" | "hpp")) => '\u{e61d}',
        (_, Some("java")) => '\u{e738}',
        (_, Some("html")) => '\u{e736}',
        (_, Some("css" | "scss")) => '\u{e749}',
        (_, Some("md" | "markdown")) => '\u{f48a}',
        (_, Some("json" | "jsonc")) => '\u{e60b}',
        (_, Some("pdf")) => '\u{f1c1}',
        (_, Some("lock")) => '\u{f023}',
        (Category::Config, _) => '\u{e615}',
        (Category::Code | Category::Executable, _) => '\u{f489}',
        (Category::Image, _) => '\u{f1c5}',
        (Category::Video, _) => '\u{f1c8}',
        (Category::Audio, _) => '\u{f1c7}',
        (Category::Archive, _) => '\u{f1c6}',
        (Category::Document, _) => '\u{f15c}',
        _ => '\u{f15b}',
    }
}

/// How an icon is drawn.
#[derive(Debug, Clone, Copy)]
pub struct Style {
    pub icons: Icons,
    pub colors: bool,
    pub utf8: bool,
}

/// The icon lines for `entry`, at most `width` wide and `height` tall. `dir_sgr` is
/// the directory colour from `LS_COLORS` (SGR parameters, may be empty).
pub fn icon(entry: &Entry, style: Style, width: usize, height: usize, dir_sgr: &str) -> Vec<Line> {
    if width < 4 || height < 2 {
        return Vec::new();
    }
    let cat = category(entry);
    match style.icons {
        Icons::None => Vec::new(),
        Icons::Nerd => {
            let color = if cat == Category::Folder && !dir_sgr.is_empty() {
                dir_sgr.to_string()
            } else {
                format!("9{}", cat.color())
            };
            let glyph = nerd_glyph(entry).to_string();
            let mut lines = vec![(paint(&glyph, &color, style.colors), 1)];
            let label = badge_label(entry, width);
            if cat != Category::Folder && !label.is_empty() {
                let w = label.width();
                lines.push((paint(&label, "2", style.colors), w));
            }
            lines
        }
        Icons::Unicode => {
            let w = width.min(10);
            let h = height.min(4);
            if cat == Category::Folder {
                folder(w, h, style, dir_sgr)
            } else {
                badge(&badge_label(entry, w - 2), cat, w.min(8), h, style)
            }
        }
    }
}

fn paint(text: &str, sgr: &str, colors: bool) -> String {
    if colors && !sgr.is_empty() {
        format!("\x1b[{sgr}m{text}\x1b[0m")
    } else {
        text.to_string()
    }
}

fn folder(w: usize, h: usize, style: Style, dir_sgr: &str) -> Vec<Line> {
    let tab = (w * 2 / 5).max(2);
    if style.colors && style.utf8 {
        // Colour only (no bold/underline), so the block characters stay solid.
        let fg = dir_sgr
            .split(';')
            .find(|p| matches!(p.parse::<u8>(), Ok(30..=37 | 90..=97)))
            .unwrap_or("34");
        let mut lines = vec![format!("{}{}", "▄".repeat(tab), " ".repeat(w - tab))];
        lines.extend((0..h.saturating_sub(2)).map(|_| "█".repeat(w)));
        lines.push("▀".repeat(w));
        return lines
            .into_iter()
            .map(|l| (paint(&l, fg, true), w))
            .collect();
    }
    let (tl, tr, bl, br, horiz, vert) = corners(style.utf8);
    let mut lines = vec![format!(
        "{tl}{}{tr}{}",
        horiz.repeat(tab - 2),
        " ".repeat(w - tab)
    )];
    lines.push(format!(
        "{vert}{}{bl}{}{tr}",
        " ".repeat(tab - 2),
        horiz.repeat(w - tab - 1)
    ));
    lines.extend((0..h.saturating_sub(3)).map(|_| format!("{vert}{}{vert}", " ".repeat(w - 2))));
    lines.push(format!("{bl}{}{br}", horiz.repeat(w - 2)));
    lines.into_iter().map(|l| (l, w)).collect()
}

fn corners(
    utf8: bool,
) -> (
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
) {
    if utf8 {
        ("╭", "╮", "╰", "╯", "─", "│")
    } else {
        ("+", "+", "+", "+", "-", "|")
    }
}

fn centered(label: &str, width: usize) -> String {
    let lw = label.width().min(width);
    let left = (width - lw) / 2;
    format!(
        "{}{label}{}",
        " ".repeat(left),
        " ".repeat(width - lw - left)
    )
}

fn badge(label: &str, cat: Category, w: usize, h: usize, style: Style) -> Vec<Line> {
    let label_row = (h - 1) / 2;
    if style.colors && style.utf8 {
        let c = cat.color();
        let fg = format!("3{c}");
        let text_fg = if matches!(c, 3 | 6 | 7) { "30" } else { "97" };
        let body = format!("1;4{c};{text_fg}");
        let mut lines = vec![(paint(&"▄".repeat(w), &fg, true), w)];
        for row in 1..h - 1 {
            let text = if row == label_row.max(1) {
                centered(label, w)
            } else {
                " ".repeat(w)
            };
            lines.push((paint(&text, &body, true), w));
        }
        lines.push((paint(&"▀".repeat(w), &fg, true), w));
        return lines;
    }
    let (tl, tr, bl, br, horiz, vert) = corners(style.utf8);
    let mut lines = vec![(format!("{tl}{}{tr}", horiz.repeat(w - 2)), w)];
    for row in 1..h - 1 {
        let text = if row == label_row.max(1) {
            centered(label, w - 2)
        } else {
            " ".repeat(w - 2)
        };
        lines.push((format!("{vert}{text}{vert}"), w));
    }
    lines.push((format!("{bl}{}{br}", horiz.repeat(w - 2)), w));
    lines
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

    const PLAIN: Style = Style {
        icons: Icons::Unicode,
        colors: false,
        utf8: true,
    };

    #[test]
    fn categories() {
        assert_eq!(category(&entry("src", Kind::Dir)), Category::Folder);
        assert_eq!(category(&file("main.rs")), Category::Code);
        assert_eq!(category(&file("Cargo.TOML")), Category::Config);
        assert_eq!(category(&file("a.tar.gz")), Category::Archive);
        assert_eq!(category(&file("README")), Category::Other);
        assert_eq!(
            category(&entry("run", Kind::File { executable: true })),
            Category::Executable
        );
        assert_eq!(category(&entry("s", Kind::Socket)), Category::Special);
    }

    #[test]
    fn badge_labels() {
        assert_eq!(badge_label(&file("main.rs"), 4), "RS");
        assert_eq!(badge_label(&file("Cargo.toml"), 4), "TOML");
        assert_eq!(badge_label(&file("data.jsonc"), 4), "JSON");
        assert_eq!(badge_label(&file(".bashrc"), 4), "");
        assert_eq!(badge_label(&file("x.\x1b[31m"), 4), "31M");
        assert_eq!(
            badge_label(&entry("tool", Kind::File { executable: true }), 4),
            "EXE"
        );
        assert_eq!(badge_label(&entry("p", Kind::Pipe), 4), "FIFO");
    }

    #[test]
    fn unicode_badge_without_colours_is_an_outline() {
        let lines = icon(&file("main.rs"), PLAIN, 16, 8, "");
        let text: Vec<&str> = lines.iter().map(|(l, _)| l.as_str()).collect();
        assert_eq!(text, ["╭──────╮", "│  RS  │", "│      │", "╰──────╯"]);
        assert!(lines.iter().all(|(l, w)| l.width() == *w));
    }

    #[test]
    fn folder_shapes() {
        let dir = entry("src", Kind::Dir);
        let lines = icon(&dir, PLAIN, 16, 8, "");
        assert!(lines.iter().all(|(l, w)| l.width() == *w && *w == 10));
        let ascii = Style {
            utf8: false,
            ..PLAIN
        };
        let lines = icon(&dir, ascii, 16, 8, "");
        assert!(lines.iter().all(|(l, _)| l.is_ascii()));
        let coloured = Style {
            colors: true,
            ..PLAIN
        };
        let lines = icon(&dir, coloured, 16, 8, "01;35");
        assert!(lines[0].0.starts_with("\x1b[35m▄"), "{:?}", lines[0].0);
    }

    #[test]
    fn coloured_badge_carries_label() {
        let style = Style {
            colors: true,
            ..PLAIN
        };
        let lines = icon(&file("Cargo.toml"), style, 16, 8, "");
        assert_eq!(lines.len(), 4);
        assert!(lines[1].0.contains("TOML"));
        assert!(lines.iter().all(|(_, w)| *w == 8));
    }

    #[test]
    fn nerd_and_none() {
        let nerd = Style {
            icons: Icons::Nerd,
            ..PLAIN
        };
        let lines = icon(&file("main.rs"), nerd, 16, 8, "");
        assert_eq!(lines[0].0, "\u{e7a8}");
        assert_eq!(lines[1].0, "RS");
        let none = Style {
            icons: Icons::None,
            ..PLAIN
        };
        assert!(icon(&file("main.rs"), none, 16, 8, "").is_empty());
    }
}
