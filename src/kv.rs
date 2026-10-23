//! Aligned "key  value" tables for metadata views (audio tags, video streams, ...).

use std::io::{self, Write};

use crate::render::sanitize_str;

/// Writes a bold title line followed by rows with the keys aligned and dimmed.
/// Values come from files, so they are sanitized; empty values are skipped.
pub fn write_table<W: Write>(
    out: &mut W,
    title: &str,
    rows: &[(&str, String)],
    colors: bool,
) -> io::Result<()> {
    let (bold, dim, reset) = if colors {
        ("\x1b[1m", "\x1b[2m", "\x1b[0m")
    } else {
        ("", "", "")
    };
    writeln!(out, "{bold}{}{reset}", sanitize_str(title))?;
    let width = rows
        .iter()
        .filter(|(_, v)| !v.is_empty())
        .map(|(k, _)| k.chars().count())
        .max()
        .unwrap_or(0);
    for (key, value) in rows.iter().filter(|(_, v)| !v.is_empty()) {
        writeln!(out, "  {dim}{key:<width$}{reset}  {}", sanitize_str(value))?;
    }
    Ok(())
}
