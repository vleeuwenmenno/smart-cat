//! Tiles view: a grid of thumbnail boxes with the file name centred under each, like a
//! file manager's icon view.
//!
//! Text methods (Unicode blocks, ASCII) build every row line by line. kitty and sixel
//! first print the row as text (icons, names, blank space where images go), which
//! also reserves the lines, then move back up and draw each image between a cursor
//! save and restore, so nothing depends on where a terminal leaves the cursor.

use std::io::{self, Write};
use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use image::DynamicImage;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::icons::{self, Line};
use super::thumbs::{self, Cache, Target};
use super::{Entry, Kind, Options, Palette, safe_name};
use crate::image::{Drawn, Method, Screen, ShowOptions};

/// Columns between two tiles.
pub const GAP: usize = 2;

/// Most threads making thumbnails at once.
const MAX_THREADS: usize = 8;

/// Number of tiles per row: as many `tile_width` tiles (with gaps) as fit in `width`,
/// at least one.
pub fn columns(width: usize, tile_width: usize) -> usize {
    ((width + GAP) / (tile_width.max(1) + GAP)).max(1)
}

/// A name fitted to `width` columns: sanitized, shortened with `…` when too long
/// (keeping a short extension), returned with its display width.
pub fn fit_name(name: &str, width: usize) -> (String, usize) {
    let name = safe_name(name);
    let full = name.width();
    if full <= width {
        return (name, full);
    }
    if width == 0 {
        return (String::new(), 0);
    }
    // Keep ".ext" when it is short and leaves room for a few characters before it.
    let tail = name
        .rfind('.')
        .filter(|&i| i > 0)
        .map(|i| &name[i..])
        .filter(|ext| ext.width() <= 5 && ext.width() + 4 <= width)
        .unwrap_or("");
    let head_room = width - 1 - tail.width();
    let mut head = String::new();
    let mut used = 0;
    for c in name.chars() {
        let cw = c.width().unwrap_or(0);
        if used + cw > head_room {
            break;
        }
        head.push(c);
        used += cw;
    }
    let text = format!("{head}…{tail}");
    let w = used + 1 + tail.width();
    (text, w)
}

/// Spaces before and after a `text_width` wide text centred in `width`.
pub fn center(text_width: usize, width: usize) -> (usize, usize) {
    let free = width.saturating_sub(text_width);
    (free / 2, free - free / 2)
}

/// What a tile shows above its name.
enum Art {
    Image(Drawn),
    Text(Vec<Line>),
}

impl Art {
    fn size(&self) -> (usize, usize) {
        match self {
            Art::Image(d) => (usize::from(d.cols), usize::from(d.rows)),
            Art::Text(lines) => (
                lines.iter().map(|(_, w)| *w).max().unwrap_or(0),
                lines.len(),
            ),
        }
    }
}

/// Renders `entries` of directory `dir` as tiles.
pub fn render<W: Write>(
    dir: &Path,
    entries: &[Entry],
    opts: &Options,
    palette: &Palette,
    out: &mut W,
) -> io::Result<()> {
    if entries.is_empty() {
        return Ok(());
    }
    let t = &opts.tiles;
    let screen = t.screen.as_ref().map(usable_screen);
    let tile_w = usize::from(t.size.0).clamp(4, opts.width.max(4));
    let mut tile_h = usize::from(t.size.1).max(2);
    if let Some(s) = &screen
        && s.method.is_pixel()
    {
        // The row must fit on the screen, or its top scrolls away before the images
        // are drawn into it.
        tile_h = tile_h.min(usize::from(s.rows).saturating_sub(3).max(2));
    }
    let cols = columns(opts.width, tile_w);

    let thumbs = if t.thumbnails
        && let Some(screen) = &screen
    {
        make_thumbnails(
            dir,
            entries,
            t.max_thumbnails,
            t.cache,
            screen,
            (tile_w, tile_h),
        )
    } else {
        Vec::new()
    };
    let dir_sgr = palette.style(&Entry {
        name: String::new(),
        kind: Kind::Dir,
        size: 0,
        modified: None,
        link_target: None,
    });
    let style = icons::Style {
        icons: t.icons,
        colors: opts.colors,
        utf8: t.utf8,
    };
    let mut thumbs = thumbs.into_iter().map(Some).collect::<Vec<_>>();
    let mut arts: Vec<Art> = Vec::with_capacity(entries.len());
    for (i, entry) in entries.iter().enumerate() {
        match thumbs.get_mut(i).and_then(Option::take).flatten() {
            Some(drawn) => arts.push(Art::Image(drawn)),
            None => arts.push(Art::Text(icons::icon(
                entry, style, tile_w, tile_h, &dir_sgr,
            ))),
        }
    }

    let pixel = screen.as_ref().is_some_and(|s| s.method.is_pixel());
    for (row, start) in (0..entries.len()).step_by(cols).enumerate() {
        let end = (start + cols).min(entries.len());
        if row > 0 {
            out.write_all(b"\n")?;
        }
        write_row(
            &entries[start..end],
            &arts[start..end],
            opts,
            palette,
            (tile_w, tile_h),
            pixel,
            out,
        )?;
    }
    Ok(())
}

/// The screen used for thumbnails. Sixel needs the cell size to know which cells an
/// image covers; without it, tiles use Unicode blocks (or ASCII) instead.
fn usable_screen(screen: &Screen) -> Screen {
    let mut s = screen.clone();
    if s.method == Method::Sixel && !s.cell_known {
        s.method = if s.colors {
            Method::HalfBlocks
        } else {
            Method::Ascii
        };
    }
    s
}

fn write_row<W: Write>(
    entries: &[Entry],
    arts: &[Art],
    opts: &Options,
    palette: &Palette,
    (tile_w, tile_h): (usize, usize),
    pixel: bool,
    out: &mut W,
) -> io::Result<()> {
    let n = entries.len();
    // Art lines, bottom-aligned in the box so every tile's art sits on the name.
    for line in 0..tile_h {
        let mut text = String::new();
        let mut pending = 0; // spaces not written yet (dropped at the end of the line)
        for (i, art) in arts.iter().enumerate() {
            let (w, h) = art.size();
            let (w, h) = (w.min(tile_w), h.min(tile_h));
            let top = tile_h - h;
            let (left, _) = center(w, tile_w);
            let cell = match art {
                Art::Text(lines) if line >= top => lines.get(line - top).map(|(s, _)| s.as_bytes()),
                Art::Image(d) if !pixel && line >= top => d.lines().get(line - top).copied(),
                _ => None,
            };
            match cell {
                Some(bytes) => {
                    text.push_str(&" ".repeat(pending + left));
                    text.push_str(&String::from_utf8_lossy(bytes));
                    pending = tile_w - left - w;
                }
                None => pending += tile_w,
            }
            if i + 1 < n {
                pending += GAP;
            }
        }
        text.push('\n');
        out.write_all(text.as_bytes())?;
    }
    // Names.
    let mut text = String::new();
    let mut pending = 0;
    for (i, entry) in entries.iter().enumerate() {
        let (name, w) = fit_name(&entry.name, tile_w);
        let (left, right) = center(w, tile_w);
        text.push_str(&" ".repeat(pending + left));
        text.push_str(&super::paint(&name, &palette.style(entry), opts.colors));
        pending = right + if i + 1 < n { GAP } else { 0 };
    }
    text.push('\n');
    out.write_all(text.as_bytes())?;

    if pixel {
        let images: Vec<(usize, &Drawn)> = arts
            .iter()
            .enumerate()
            .filter_map(|(i, a)| match a {
                Art::Image(d) => Some((i, d)),
                Art::Text(_) => None,
            })
            .collect();
        if images.is_empty() {
            return Ok(());
        }
        let height = tile_h + 1;
        write!(out, "\x1b[{height}A\x1b7")?;
        for (i, drawn) in images {
            let (w, h) = (
                usize::from(drawn.cols).min(tile_w),
                usize::from(drawn.rows).min(tile_h),
            );
            let x = i * (tile_w + GAP) + center(w, tile_w).0;
            let down = tile_h - h;
            out.write_all(b"\x1b8")?;
            if down > 0 {
                write!(out, "\x1b[{down}B")?;
            }
            if x > 0 {
                write!(out, "\x1b[{x}C")?;
            }
            out.write_all(&drawn.data)?;
        }
        write!(out, "\x1b8\x1b[{height}B")?;
    }
    Ok(())
}

/// Thumbnails for the first `max` entries that can have one, drawn for `screen` in
/// parallel. Index `i` of the result belongs to `entries[i]`.
fn make_thumbnails(
    dir: &Path,
    entries: &[Entry],
    max: usize,
    use_cache: bool,
    screen: &Screen,
    (tile_w, tile_h): (usize, usize),
) -> Vec<Option<Drawn>> {
    let base = std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
    let jobs: Vec<(usize, thumbs::Source)> = entries
        .iter()
        .enumerate()
        .filter(|(_, e)| {
            matches!(
                e.kind,
                Kind::File { .. }
                    | Kind::Symlink {
                        to_dir: false,
                        broken: false
                    }
            )
        })
        .filter_map(|(i, e)| thumbs::source_for(&e.name).map(|s| (i, s)))
        .take(max)
        .collect();
    let mut results: Vec<Option<Drawn>> = (0..entries.len()).map(|_| None).collect();
    if jobs.is_empty() {
        return results;
    }
    let cache = Cache::new(use_cache);
    let show = ShowOptions {
        max_cols: Some(tile_w as u16),
        max_rows: Some(tile_h as u16),
        pin_rows: screen.method == Method::Kitty,
        ..Default::default()
    };
    let next = AtomicUsize::new(0);
    let done = Mutex::new(Vec::with_capacity(jobs.len()));
    let threads = std::thread::available_parallelism()
        .map_or(4, |n| n.get())
        .min(MAX_THREADS)
        .min(jobs.len());
    std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| {
                loop {
                    let k = next.fetch_add(1, Ordering::Relaxed);
                    let Some(&(i, source)) = jobs.get(k) else {
                        break;
                    };
                    let drawn = one_thumbnail(&base.join(&entries[i].name), source, &cache)
                        .and_then(|img| {
                            crate::image::draw(screen, &DynamicImage::ImageRgba8(img), &show).ok()
                        });
                    if let Ok(mut done) = done.lock() {
                        done.push((i, drawn));
                    }
                }
            });
        }
    });
    for (i, drawn) in done.into_inner().unwrap_or_default() {
        results[i] = drawn;
    }
    results
}

fn one_thumbnail(path: &Path, source: thumbs::Source, cache: &Cache) -> Option<image::RgbaImage> {
    // Follows symlinks: the thumbnail is of the file the link points to.
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() {
        return None;
    }
    let target = Target {
        path,
        size: meta.len(),
        modified: meta.modified().ok()?,
    };
    thumbs::thumbnail(cache, &target, source)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tile_columns() {
        assert_eq!(columns(80, 16), 4); // 4 * 16 + 3 * 2 = 70
        assert_eq!(columns(87, 16), 4);
        assert_eq!(columns(5 * 16 + 4 * 2, 16), 5);
        assert_eq!(columns(10, 16), 1);
        assert_eq!(columns(0, 0), 1);
    }

    #[test]
    fn names_fit_and_keep_extensions() {
        assert_eq!(fit_name("short.rs", 16), ("short.rs".into(), 8));
        let (text, w) = fit_name("a-very-long-file-name.png", 16);
        assert_eq!(text, "a-very-long….png");
        assert_eq!(w, 16);
        assert_eq!(text.width(), 16);
        // No extension, or one too long to keep.
        assert_eq!(fit_name("abcdefghijklmnopqrstuvwxyz", 8).0, "abcdefg…");
        assert_eq!(fit_name("archive.tar-backup-old", 8).0, "archive…");
    }

    #[test]
    fn wide_characters_are_cut_by_display_width() {
        // Each CJK character is two columns wide.
        let (text, w) = fit_name("日本語のファイル名.txt", 12);
        assert!(w <= 12, "{text} {w}");
        assert_eq!(text.width(), w);
        assert_eq!((text.as_str(), w), ("日本語….txt", 11));
        let (text, w) = fit_name("日本語日本語", 6);
        assert_eq!((text.as_str(), w), ("日本…", 5));
        let (left, right) = center(5, 6);
        assert_eq!((left, right), (0, 1));
        assert_eq!(center(2, 16), (7, 7));
        assert_eq!(center(20, 16), (0, 0));
    }

    #[test]
    fn weird_names_are_sanitized() {
        let (text, _) = fit_name("evil\x1b]52;c;AAAA\x07\n.png", 40);
        assert!(!text.contains('\x1b'));
        assert!(!text.contains('\x07'));
        assert!(!text.contains('\n'));
        let (text, _) = fit_name("rtl\u{202e}gnp.exe", 40);
        assert!(!text.contains('\u{202e}'));
    }
}
