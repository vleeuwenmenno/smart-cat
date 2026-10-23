//! Font view: an info table per face and a rendered specimen.
//!
//! TrueType, OpenType (CFF/CFF2), collections (`.ttc`/`.otc`, every face listed),
//! WOFF and WOFF2. Web fonts are unwrapped to a plain sfnt first ([`woff`]); facts
//! come from skrifa/read-fonts ([`info`]); the specimen is drawn by resvg from an SVG
//! `<text>` document whose only font is the one shown ([`specimen`]).
//!
//! Font files are untrusted. All parsing and drawing happens in a helper process
//! ([`helper`], `scat --internal-font`) under the PDF sandbox limits: a parser panic or
//! a runaway font becomes one error line and scat goes on. The helper returns the
//! table rows as JSON and the specimen as a PNG; names are sanitized when printed.

mod helper;
mod info;
mod specimen;
mod woff;

use std::ffi::OsString;
use std::io::{self, Read, Write};
use std::time::Duration;

use image::RgbaImage;
use serde::{Deserialize, Serialize};

use crate::image::{Screen, ShowOptions};
use crate::input::{Input, InputKind};
use crate::sandbox;

/// Hidden first argument that turns scat into the font helper process.
pub const FLAG: &str = "--internal-font";

/// Font files larger than this are not read.
pub const MAX_FONT_BYTES: u64 = 128 << 20;
/// Faces of a collection listed in detail.
const MAX_FACES: usize = 64;

/// Runs the helper when scat was started as one; `None` for a normal run.
pub fn internal_main() -> Option<i32> {
    let mut args = std::env::args_os().skip(1);
    if args.next()? != FLAG {
        return None;
    }
    Some(helper::main(args.collect()))
}

/// Whether `auto` sends a file to the font view: an sfnt, collection or WOFF header
/// that looks sane (a table count in range and a printable first table tag).
pub fn wants(head: &[u8]) -> bool {
    let Some(magic) = head.get(..4) else {
        return false;
    };
    let u16_at = |i: usize| head.get(i..i + 2).map(|b| u16::from_be_bytes([b[0], b[1]]));
    let u32_at = |i: usize| {
        head.get(i..i + 4)
            .map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    };
    match magic {
        b"\0\x01\0\0" | b"OTTO" | b"true" => {
            let tables = u16_at(4).unwrap_or(0);
            let tag = head.get(12..16).unwrap_or(&[]);
            (1..=300).contains(&tables)
                && tag.len() == 4
                && tag.iter().all(|&b| b.is_ascii_graphic() || b == b' ')
        }
        b"ttcf" => {
            matches!(u32_at(4), Some(0x0001_0000 | 0x0002_0000))
                && u32_at(8).is_some_and(|n| (1..=10_000).contains(&n))
        }
        b"wOFF" | b"wOF2" => matches!(
            head.get(4..8),
            Some(b"\0\x01\0\0" | b"OTTO" | b"true" | b"ttcf")
        ),
        _ => false,
    }
}

/// What the helper reports: one table per face (at most [`MAX_FACES`]).
#[derive(Debug, Serialize, Deserialize)]
struct Report {
    container: String,
    faces: usize,
    tables: Vec<FaceTable>,
}

#[derive(Debug, Serialize, Deserialize)]
struct FaceTable {
    label: String,
    rows: Vec<(String, String)>,
}

pub struct Options<'a> {
    pub name: &'a str,
    pub colors: bool,
    pub utf8: bool,
    /// Where the specimen is drawn; `None` for the table only.
    pub screen: Option<Screen>,
}

/// Runs the helper with `data` on its standard input.
fn run_helper(data: Vec<u8>, args: &[String], timeout: Duration) -> Result<Vec<u8>, String> {
    let cmd = sandbox::helper(FLAG, args.iter().map(OsString::from));
    let limits = sandbox::Limits {
        timeout,
        stdin: sandbox::Input::Bytes(&data),
        ..sandbox::Limits::default()
    };
    sandbox::run_with(cmd, limits).map_err(|e| e.to_string())
}

/// Renders the font view. Errors become `cannot read font: ...` before anything is
/// written.
pub fn render<W: Write>(out: &mut W, input: &mut Input, opts: &Options) -> io::Result<()> {
    let mut data = Vec::new();
    (&mut input.reader)
        .take(MAX_FONT_BYTES + 1)
        .read_to_end(&mut data)?;
    if data.len() as u64 > MAX_FONT_BYTES {
        return Err(io::Error::other(format!(
            "font file larger than {} MiB, not read",
            MAX_FONT_BYTES >> 20
        )));
    }
    let size = match input.kind {
        InputKind::Regular { size } => size,
        _ => data.len() as u64,
    };
    // The specimen runs in its own helper, so a font that breaks the shaper or the
    // rasteriser still gets its table.
    let specimen_data = opts.screen.as_ref().map(|_| data.clone());
    let args = [
        "info".to_string(),
        if opts.utf8 { "1" } else { "0" }.to_string(),
    ];
    let bad = |e: String| io::Error::other(format!("cannot read font: {e}"));
    let json = run_helper(data, &args, sandbox::TIMEOUT).map_err(bad)?;
    let report: Report = serde_json::from_slice(&json).map_err(|e| bad(e.to_string()))?;

    let mut buf = Vec::new();
    let size_text = crate::util::human_size(size);
    fn rows(t: &FaceTable) -> Vec<(&str, String)> {
        t.rows
            .iter()
            .map(|(k, v)| (k.as_str(), v.clone()))
            .collect()
    }
    if let [table] = &report.tables[..] {
        let mut all = vec![("format", report.container.clone())];
        all.extend(rows(table));
        all.push(("file size", size_text));
        crate::kv::write_table(&mut buf, opts.name, &all, opts.colors)?;
    } else {
        let head = [
            ("format", report.container.clone()),
            ("faces", report.faces.to_string()),
            ("file size", size_text),
        ];
        crate::kv::write_table(&mut buf, opts.name, &head, opts.colors)?;
        for (i, table) in report.tables.iter().enumerate() {
            buf.write_all(b"\n")?;
            let title = format!("face {i}: {}", table.label);
            crate::kv::write_table(&mut buf, &title, &rows(table), opts.colors)?;
        }
        if report.faces > report.tables.len() {
            writeln!(buf, "\n… {} more faces", report.faces - report.tables.len())?;
        }
    }
    out.write_all(&buf)?;

    let (Some(screen), Some(data)) = (&opts.screen, specimen_data) else {
        return Ok(());
    };
    out.write_all(b"\n")?;
    out.flush()?;
    // Pixel methods get the nominal sizes scaled to the cell height (16 px cells = 1x).
    let scale = if screen.method.is_pixel() {
        (f32::from(screen.cell.1) / 16.0).clamp(1.0, 3.0)
    } else {
        1.0
    };
    let args = ["specimen".to_string(), format!("{scale:.2}")];
    let drawn = run_helper(data, &args, sandbox::TIMEOUT).and_then(|png| {
        crate::image::thumbnail(png, false, u32::MAX).map(image::DynamicImage::ImageRgba8)
    });
    match drawn {
        Ok(img) => crate::image::show(out, screen, &img, &ShowOptions::default()),
        Err(e) => {
            let note = crate::render::sanitize_str(&format!("no specimen: {e}"));
            if opts.colors {
                writeln!(out, "\x1b[2m{note}\x1b[22m")
            } else {
                writeln!(out, "{note}")
            }
        }
    }
}

/// A directory thumbnail: a big `Aa` in the first face, `side` pixels square, made by
/// the helper within `timeout`.
pub fn thumbnail(data: Vec<u8>, side: u32, timeout: Duration) -> Option<RgbaImage> {
    let png = run_helper(data, &["thumb".to_string(), side.to_string()], timeout).ok()?;
    crate::image::thumbnail(png, false, side).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sniffs_font_headers() {
        let mut ttf = b"\0\x01\0\0\0\x0a\0\x80\0\x03\0\x20".to_vec();
        ttf.extend_from_slice(b"GDEF");
        assert!(wants(&ttf));
        assert!(wants(b"ttcf\0\x01\0\0\0\0\0\x02"));
        assert!(wants(b"wOF2OTTO"));
        assert!(!wants(b"wOF2junk"));
        // The TrueType magic alone is not enough.
        assert!(!wants(b"\0\x01\0\0\0\0\0\0\0\0\0\0\x01\x02\x03\x04"));
        assert!(!wants(b"OTTO"));
    }
}
