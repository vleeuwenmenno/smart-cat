//! The font helper process, with the font file on standard input:
//!
//! - `scat --internal-font info UTF8`: the info tables as JSON,
//! - `scat --internal-font specimen SCALE`: the specimen of the first face as a PNG,
//! - `scat --internal-font thumb SIDE`: a directory thumbnail as a PNG.
//!
//! Only this code parses and shapes fonts (read-fonts, skrifa, harfrust, resvg). It is
//! started by [`crate::sandbox`] with the same limits as the PDF helper, since a
//! malformed font can make a parser panic, which aborts a release build. Errors go to
//! stderr as one line and exit with status 2.

use std::ffi::OsString;
use std::io::{Read, Write};
use std::sync::Arc;

use image::RgbaImage;
use skrifa::raw::{FileRef, FontRef, TableProvider};

use super::{FaceTable, MAX_FACES, MAX_FONT_BYTES, Report, info, specimen, woff};

/// Entry point; `args` are the arguments after [`super::FLAG`].
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
    let text = |i: usize| args.get(i).and_then(|a| a.to_str()).ok_or("usage error");
    let mut data = Vec::new();
    std::io::stdin()
        .lock()
        .take(MAX_FONT_BYTES + 1)
        .read_to_end(&mut data)
        .map_err(|e| crate::util::io_message(&e))?;
    if data.len() as u64 > MAX_FONT_BYTES {
        return Err("font too large".into());
    }
    match text(0)? {
        "info" => info(data, text(1)? == "1"),
        "specimen" => {
            let scale: f32 = text(1)?.parse().map_err(|_| "usage error")?;
            let loaded = load(data)?;
            let face = info::describe(faces(&loaded.sfnt)?.first().ok_or("no face")?);
            png(&specimen::render(loaded.sfnt.clone(), 0, &face, scale)?)
        }
        "thumb" => {
            let side: u32 = text(1)?.parse().map_err(|_| "usage error")?;
            let img = thumbnail(data, side).ok_or("no thumbnail")?;
            png(&img)
        }
        _ => Err("usage error".into()),
    }
}

/// A font file unwrapped to sfnt bytes, with a description of its container.
struct Loaded {
    sfnt: Arc<Vec<u8>>,
    container: &'static str,
}

fn load(data: Vec<u8>) -> Result<Loaded, String> {
    let (sfnt, wrapper) = match woff::wrapper(&data) {
        Some(w) => (woff::to_sfnt(&data)?, Some(w)),
        None => (data, None),
    };
    let flavor = sfnt.get(..4).unwrap_or(&[]);
    let container = match (wrapper, flavor) {
        (Some(woff::Wrapper::Woff), _) => "WOFF",
        (Some(woff::Wrapper::Woff2), b"ttcf") => "WOFF2 collection",
        (Some(woff::Wrapper::Woff2), _) => "WOFF2",
        (None, b"ttcf") => "OpenType collection",
        (None, b"true") => "TrueType (Apple)",
        (None, _) => "OpenType",
    };
    Ok(Loaded {
        sfnt: Arc::new(sfnt),
        container,
    })
}

/// The faces of a font file (one for a plain font). Faces without a readable `head`
/// and `maxp` table are refused.
fn faces(sfnt: &[u8]) -> Result<Vec<FontRef<'_>>, String> {
    fn check(font: FontRef<'_>) -> Result<FontRef<'_>, String> {
        font.head().map_err(|e| format!("head table: {e}"))?;
        font.maxp().map_err(|e| format!("maxp table: {e}"))?;
        Ok(font)
    }
    let fonts = match FileRef::new(sfnt).map_err(|e| e.to_string())? {
        FileRef::Font(font) => vec![check(font)?],
        FileRef::Collection(c) => c
            .iter()
            .map(|f| f.map_err(|e| e.to_string()).and_then(check))
            .collect::<Result<Vec<_>, _>>()?,
    };
    if fonts.is_empty() {
        return Err("empty collection".into());
    }
    Ok(fonts)
}

/// The report as JSON.
fn info(data: Vec<u8>, utf8: bool) -> Result<Vec<u8>, String> {
    let loaded = load(data)?;
    let fonts = faces(&loaded.sfnt)?;
    let described: Vec<info::Face> = fonts.iter().take(MAX_FACES).map(info::describe).collect();
    let report = Report {
        container: loaded.container.to_string(),
        faces: fonts.len(),
        tables: described
            .iter()
            .map(|face| FaceTable {
                label: if face.full_name.is_empty() {
                    face.family.clone()
                } else {
                    face.full_name.clone()
                },
                rows: info::rows(face, utf8)
                    .into_iter()
                    .map(|(k, v)| (k.to_string(), v))
                    .collect(),
            })
            .collect(),
    };
    serde_json::to_vec(&report).map_err(|e| e.to_string())
}

fn thumbnail(data: Vec<u8>, side: u32) -> Option<RgbaImage> {
    let loaded = load(data).ok()?;
    let face = {
        let fonts = faces(&loaded.sfnt).ok()?;
        info::describe(fonts.first()?)
    };
    specimen::thumbnail(loaded.sfnt.clone(), 0, &face, side.clamp(16, 1024))
}

fn png(img: &RgbaImage) -> Result<Vec<u8>, String> {
    use image::ImageEncoder as _;
    let mut out = Vec::new();
    image::codecs::png::PngEncoder::new_with_quality(
        &mut out,
        image::codecs::png::CompressionType::Fast,
        image::codecs::png::FilterType::Adaptive,
    )
    .write_image(
        img,
        img.width(),
        img.height(),
        image::ExtendedColorType::Rgba8,
    )
    .map_err(|e| e.to_string())?;
    Ok(out)
}
