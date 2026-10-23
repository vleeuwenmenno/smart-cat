//! The PDF helper process: `scat --internal-pdf info FILE` and
//! `scat --internal-pdf render FILE PAGE MAX_WIDTH MAX_HEIGHT`.
//!
//! Only this code runs hayro. It is started by [`super::sandbox::run`] with resource
//! limits; errors go to stderr as one line and exit with status 2.

use std::ffi::OsString;
use std::io::Write;
use std::sync::Arc;

use hayro::hayro_interpret::InterpreterSettings;
use hayro::hayro_syntax::{DecryptionError, LoadPdfError, Pdf};
use hayro::vello_cpu::color::palette::css::WHITE;
use hayro::{PixmapSettings, RenderCache, RenderSettings};

use super::Info;

/// Largest PDF the helper reads into memory.
const MAX_FILE_BYTES: u64 = 512 * 1024 * 1024;

/// Largest rendered side in pixels (vello_cpu pixmaps are limited to u16).
const MAX_SIDE: f32 = 8192.0;

/// Entry point for the helper; `args` are the arguments after [`super::sandbox::FLAG`].
pub fn main(args: Vec<OsString>) -> i32 {
    match run(&args) {
        Ok(bytes) => {
            let mut stdout = std::io::stdout().lock();
            if stdout
                .write_all(&bytes)
                .and_then(|()| stdout.flush())
                .is_err()
            {
                return 1;
            }
            0
        }
        Err(message) => {
            eprintln!("{message}");
            2
        }
    }
}

fn run(args: &[OsString]) -> Result<Vec<u8>, String> {
    let text = |i: usize| args.get(i).and_then(|a| a.to_str());
    let number = |i: usize| -> Result<u32, String> {
        text(i)
            .and_then(|t| t.parse().ok())
            .ok_or_else(|| "usage error".to_string())
    };
    let path = args.get(1).ok_or("usage error")?;
    match text(0) {
        Some("info") => {
            let data = read(path)?;
            let info = info(data);
            serde_json::to_vec(&info).map_err(|e| e.to_string())
        }
        Some("render") => {
            let data = read(path)?;
            render(data, number(2)?, number(3)?, number(4)?)
        }
        _ => Err("usage error".into()),
    }
}

fn read(path: &OsString) -> Result<Vec<u8>, String> {
    let file = std::fs::File::open(path).map_err(|e| crate::util::io_message(&e))?;
    let mut data = Vec::new();
    std::io::Read::read_to_end(
        &mut std::io::Read::take(file, MAX_FILE_BYTES + 1),
        &mut data,
    )
    .map_err(|e| crate::util::io_message(&e))?;
    if data.len() as u64 > MAX_FILE_BYTES {
        return Err(format!(
            "larger than {} MiB, not opened",
            MAX_FILE_BYTES >> 20
        ));
    }
    Ok(data)
}

fn open(data: Vec<u8>) -> Result<Pdf, String> {
    Pdf::new(Arc::new(data)).map_err(|err| load_error(err).to_string())
}

fn load_error(err: LoadPdfError) -> &'static str {
    match err {
        LoadPdfError::Invalid => "not a valid PDF file",
        LoadPdfError::Decryption(DecryptionError::PasswordProtected) => {
            "encrypted, a password is needed"
        }
        LoadPdfError::Decryption(DecryptionError::UnsupportedAlgorithm) => {
            "encrypted with an unsupported algorithm"
        }
        LoadPdfError::Decryption(_) => "broken encryption",
    }
}

fn info(data: Vec<u8>) -> Info {
    let encrypted = mentions_encrypt(&data);
    let version = header_version(&data);
    let pdf = match Pdf::new(Arc::new(data)) {
        Ok(pdf) => pdf,
        Err(err) => {
            return Info {
                version,
                encrypted,
                error: Some(load_error(err).to_string()),
                needs_password: matches!(
                    err,
                    LoadPdfError::Decryption(DecryptionError::PasswordProtected)
                ),
                ..Info::default()
            };
        }
    };
    let pages = pdf.pages();
    let meta = pdf.metadata();
    let date = |d: &Option<hayro::hayro_syntax::object::DateTime>| {
        d.as_ref().map(|d| {
            format!(
                "{:04}-{:02}-{:02} {:02}:{:02}",
                d.year, d.month, d.day, d.hour, d.minute
            )
        })
    };
    Info {
        pages: pages.len() as u32,
        page_size: pages.first().map(|p| p.render_dimensions()),
        version,
        title: meta.title.as_deref().map(decode_text),
        author: meta.author.as_deref().map(decode_text),
        subject: meta.subject.as_deref().map(decode_text),
        creator: meta.creator.as_deref().map(decode_text),
        producer: meta.producer.as_deref().map(decode_text),
        created: date(&meta.creation_date),
        modified: date(&meta.modification_date),
        encrypted,
        needs_password: false,
        error: None,
    }
}

fn render(data: Vec<u8>, page: u32, max_w: u32, max_h: u32) -> Result<Vec<u8>, String> {
    let pdf = open(data)?;
    let pages = pdf.pages();
    let index = page.checked_sub(1).ok_or("no page 0")? as usize;
    let page = pages
        .get(index)
        .ok_or_else(|| format!("no page {}", index + 1))?;
    let (w, h) = page.render_dimensions();
    if !(w >= 1.0 && h >= 1.0) {
        return Err("empty page".into());
    }
    let scale = (max_w.max(16) as f32 / w)
        .min(max_h.max(16) as f32 / h)
        .min(MAX_SIDE / w)
        .min(MAX_SIDE / h);
    let cache = RenderCache::new();
    let pixmap = hayro::render(
        page,
        &cache,
        &InterpreterSettings::default(),
        &RenderSettings::default(),
        &PixmapSettings {
            x_scale: scale,
            y_scale: scale,
            bg_color: WHITE,
        },
    );
    let (pw, ph) = (u32::from(pixmap.width()), u32::from(pixmap.height()));
    let rgba = pixmap.take_rgba8(hayro::vello_cpu::peniko::ImageAlphaType::Alpha);
    let mut png = Vec::new();
    image::ImageEncoder::write_image(
        image::codecs::png::PngEncoder::new_with_quality(
            &mut png,
            image::codecs::png::CompressionType::Fast,
            image::codecs::png::FilterType::NoFilter,
        ),
        &rgba,
        pw,
        ph,
        image::ExtendedColorType::Rgba8,
    )
    .map_err(|e| e.to_string())?;
    Ok(png)
}

/// Whether the trailer area mentions `/Encrypt` (hayro opens files that only have an
/// owner password without saying so).
fn mentions_encrypt(data: &[u8]) -> bool {
    const WINDOW: usize = 64 * 1024;
    let tail = &data[data.len().saturating_sub(WINDOW)..];
    let head = &data[..data.len().min(WINDOW)];
    [head, tail]
        .iter()
        .any(|part| part.windows(8).any(|w| w == b"/Encrypt"))
}

fn header_version(data: &[u8]) -> Option<String> {
    let head = &data[..data.len().min(1024)];
    let pos = head.windows(5).position(|w| w == b"%PDF-")?;
    let version: String = head[pos + 5..]
        .iter()
        .take_while(|b| b.is_ascii_digit() || **b == b'.')
        .take(4)
        .map(|&b| b as char)
        .collect();
    (!version.is_empty()).then_some(version)
}

/// Decodes a PDF text string: UTF-16BE or UTF-8 with a byte order mark, otherwise
/// UTF-8 if valid, otherwise PDFDocEncoding (read as Latin-1, close enough for display).
pub fn decode_text(bytes: &[u8]) -> String {
    const MAX_CHARS: usize = 300;
    let text = if let Some(rest) = bytes.strip_prefix(b"\xfe\xff") {
        let units: Vec<u16> = rest
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&c| u16::from_be_bytes(c))
            .collect();
        String::from_utf16_lossy(&units)
    } else if let Some(rest) = bytes.strip_prefix(b"\xef\xbb\xbf") {
        String::from_utf8_lossy(rest).into_owned()
    } else if let Ok(text) = std::str::from_utf8(bytes) {
        text.to_string()
    } else {
        bytes.iter().map(|&b| b as char).collect()
    };
    text.trim().chars().take(MAX_CHARS).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_pdf_text_strings() {
        assert_eq!(decode_text(b"Plain"), "Plain");
        assert_eq!(decode_text(b"\xfe\xff\x00H\x00\xe9"), "Hé");
        assert_eq!(decode_text(b"caf\xe9"), "café");
        assert_eq!(decode_text(b"\xef\xbb\xbf ok "), "ok");
    }

    #[test]
    fn reads_header_version_and_encrypt_marker() {
        assert_eq!(header_version(b"%PDF-1.7\n%..."), Some("1.7".into()));
        assert_eq!(header_version(b"nothing"), None);
        assert!(mentions_encrypt(b"trailer << /Encrypt 5 0 R >>"));
        assert!(!mentions_encrypt(b"trailer << /Root 1 0 R >>"));
    }
}
