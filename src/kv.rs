//! Aligned "key  value" tables for metadata views (audio tags, video streams, ...).

use std::io::{self, Write};

use unicode_width::UnicodeWidthChar;

use crate::render::sanitize_str;

/// Writes a bold title line followed by rows with the keys aligned and dimmed.
/// Values come from files, so they are sanitized; empty values are skipped.
pub fn write_table<W: Write>(
    out: &mut W,
    title: &str,
    rows: &[(&str, String)],
    colors: bool,
) -> io::Result<()> {
    for line in table_lines(title, rows, colors, None) {
        out.write_all(&line)?;
        out.write_all(b"\n")?;
    }
    Ok(())
}

/// The lines of [`write_table`], without newlines. With `max_width`, every line is cut
/// to that many columns (ending in `…`), for tables drawn beside an image.
pub fn table_lines(
    title: &str,
    rows: &[(&str, String)],
    colors: bool,
    max_width: Option<usize>,
) -> Vec<Vec<u8>> {
    let (bold, dim, reset) = if colors {
        ("\x1b[1m", "\x1b[2m", "\x1b[0m")
    } else {
        ("", "", "")
    };
    let fit = |text: &str, used: usize| match max_width {
        Some(max) => truncate(text, max.saturating_sub(used)),
        None => text.to_string(),
    };
    let mut lines = Vec::with_capacity(rows.len() + 1);
    let title = fit(&sanitize_str(title), 0);
    lines.push(format!("{bold}{title}{reset}").into_bytes());
    let width = rows
        .iter()
        .filter(|(_, v)| !v.is_empty())
        .map(|(k, _)| k.chars().count())
        .max()
        .unwrap_or(0);
    for (key, value) in rows.iter().filter(|(_, v)| !v.is_empty()) {
        let value = fit(&sanitize_str(value), width + 4);
        lines.push(format!("  {dim}{key:<width$}{reset}  {value}").into_bytes());
    }
    lines
}

/// Cuts `text` to at most `max` columns, marking a cut with `…`.
fn truncate(text: &str, max: usize) -> String {
    let width: usize = text.chars().map(|c| c.width().unwrap_or(0)).sum();
    if width <= max {
        return text.to_string();
    }
    let mut out = String::new();
    let mut used = 0;
    for c in text.chars() {
        let w = c.width().unwrap_or(0);
        if used + w + 1 > max {
            break;
        }
        out.push(c);
        used += w;
    }
    if max > 0 {
        out.push('…');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_are_cut_to_the_width() {
        let rows = [
            ("title", "a long title that does not fit".to_string()),
            ("x", String::new()),
        ];
        let lines = table_lines("name.mp3", &rows, false, Some(20));
        let text: Vec<String> = lines
            .into_iter()
            .map(|l| String::from_utf8(l).unwrap())
            .collect();
        assert_eq!(text, ["name.mp3", "  title  a long tit…"]);
        assert_eq!(truncate("日本語", 4), "日…");
        assert_eq!(truncate("abc", 3), "abc");
    }
}
