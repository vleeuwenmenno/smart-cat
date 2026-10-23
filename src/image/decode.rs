//! Decoding: raster formats through the `image` crate with limits, SVG through resvg.

use std::io::Cursor;
use std::sync::Arc;

use image::codecs::gif::GifDecoder;
use image::codecs::png::PngDecoder;
use image::codecs::webp::WebPDecoder;
use image::{AnimationDecoder, DynamicImage, Frame, ImageFormat, ImageReader, Limits, RgbaImage};

/// Images with more pixels than this are refused before decoding.
pub const MAX_PIXELS: u64 = 100_000_000;
/// Upper bound for decoder allocations.
const MAX_ALLOC: u64 = 1024 * 1024 * 1024;

pub type Frames<'a> = Box<dyn Iterator<Item = image::ImageResult<Frame>> + 'a>;

pub enum Decoded {
    Raster {
        data: Vec<u8>,
        format: ImageFormat,
        width: u32,
        height: u32,
    },
    Svg {
        tree: Box<resvg::usvg::Tree>,
        width: f32,
        height: f32,
    },
}

impl Decoded {
    /// Reads the header (raster) or parses the document (SVG). Refuses images beyond
    /// `MAX_PIXELS` without decoding them.
    pub fn open(data: Vec<u8>, svg: bool) -> Result<Self, String> {
        if svg {
            return open_svg(&data);
        }
        let reader = ImageReader::new(Cursor::new(&data[..]))
            .with_guessed_format()
            .map_err(|e| e.to_string())?;
        let format = reader
            .format()
            .ok_or_else(|| "unknown image format".to_string())?;
        let (width, height) = reader.into_dimensions().map_err(|e| e.to_string())?;
        check_size(width, height)?;
        Ok(Decoded::Raster {
            data,
            format,
            width,
            height,
        })
    }

    pub fn size(&self) -> (u32, u32) {
        match self {
            Decoded::Raster { width, height, .. } => (*width, *height),
            Decoded::Svg { width, height, .. } => (
                width.round().max(1.0) as u32,
                height.round().max(1.0) as u32,
            ),
        }
    }

    pub fn format_name(&self) -> &'static str {
        match self {
            Decoded::Svg { .. } => "SVG",
            Decoded::Raster { format, .. } => match format {
                ImageFormat::Png => "PNG",
                ImageFormat::Jpeg => "JPEG",
                ImageFormat::Gif => "GIF",
                ImageFormat::WebP => "WebP",
                ImageFormat::Bmp => "BMP",
                ImageFormat::Ico => "ICO",
                ImageFormat::Tiff => "TIFF",
                ImageFormat::Qoi => "QOI",
                ImageFormat::Pnm => "PNM",
                ImageFormat::Tga => "TGA",
                _ => "image",
            },
        }
    }

    /// The original file bytes when they are a PNG (kitty can take them as they are).
    pub fn png_bytes(&self) -> Option<&[u8]> {
        match self {
            Decoded::Raster {
                data,
                format: ImageFormat::Png,
                ..
            } => Some(data),
            _ => None,
        }
    }

    /// Decodes the image, or the first frame of an animation.
    pub fn first_frame(&self) -> Result<DynamicImage, String> {
        match self {
            Decoded::Raster { data, format, .. } => {
                let mut reader = ImageReader::with_format(Cursor::new(&data[..]), *format);
                reader.limits(limits());
                reader.decode().map_err(|e| e.to_string())
            }
            Decoded::Svg { .. } => {
                let (w, h) = self.size();
                self.render_svg(w, h).map(DynamicImage::ImageRgba8)
            }
        }
    }

    /// The frames of an animated GIF, WebP or PNG, or `None` for still images. Frames
    /// are decoded lazily as the iterator advances.
    pub fn frames(&self) -> Result<Option<Frames<'_>>, String> {
        let Decoded::Raster { data, format, .. } = self else {
            return Ok(None);
        };
        let err = |e: image::ImageError| e.to_string();
        let cursor = Cursor::new(&data[..]);
        Ok(match format {
            ImageFormat::Gif => {
                let mut decoder = GifDecoder::new(cursor).map_err(err)?;
                image::ImageDecoder::set_limits(&mut decoder, limits()).map_err(err)?;
                Some(Box::new(decoder.into_frames()))
            }
            ImageFormat::WebP => {
                let mut decoder = WebPDecoder::new(cursor).map_err(err)?;
                if !decoder.has_animation() {
                    return Ok(None);
                }
                image::ImageDecoder::set_limits(&mut decoder, limits()).map_err(err)?;
                Some(Box::new(decoder.into_frames()))
            }
            ImageFormat::Png => {
                let decoder = PngDecoder::with_limits(cursor, limits()).map_err(err)?;
                if !decoder.is_apng().map_err(err)? {
                    return Ok(None);
                }
                Some(Box::new(decoder.apng().map_err(err)?.into_frames()))
            }
            _ => None,
        })
    }

    /// Renders an SVG at exactly `width`x`height` pixels (aspect ratio kept, centred).
    pub fn render_svg(&self, width: u32, height: u32) -> Result<RgbaImage, String> {
        let Decoded::Svg {
            tree,
            width: sw,
            height: sh,
        } = self
        else {
            return Err("not an SVG".into());
        };
        let mut pixmap = resvg::tiny_skia::Pixmap::new(width.max(1), height.max(1))
            .ok_or_else(|| "SVG too large".to_string())?;
        let scale = (width as f32 / sw).min(height as f32 / sh);
        let dx = (width as f32 - sw * scale) / 2.0;
        let dy = (height as f32 - sh * scale) / 2.0;
        let transform = resvg::tiny_skia::Transform::from_row(scale, 0.0, 0.0, scale, dx, dy);
        resvg::render(tree, transform, &mut pixmap.as_mut());
        RgbaImage::from_raw(width.max(1), height.max(1), pixmap.take_demultiplied())
            .ok_or_else(|| "SVG render failed".to_string())
    }
}

fn limits() -> Limits {
    let mut limits = Limits::default();
    limits.max_image_width = Some(1 << 16);
    limits.max_image_height = Some(1 << 16);
    limits.max_alloc = Some(MAX_ALLOC);
    limits
}

fn check_size(width: u32, height: u32) -> Result<(), String> {
    if width == 0 || height == 0 {
        return Err("image has no pixels".into());
    }
    let pixels = u64::from(width) * u64::from(height);
    if pixels > MAX_PIXELS {
        return Err(format!(
            "image too large ({width}x{height}, {} megapixels; the limit is {})",
            pixels / 1_000_000,
            MAX_PIXELS / 1_000_000
        ));
    }
    Ok(())
}

fn open_svg(data: &[u8]) -> Result<Decoded, String> {
    use resvg::usvg;
    let mut options = usvg::Options {
        // Never read other files referenced by the SVG (<image href="/path">).
        image_href_resolver: usvg::ImageHrefResolver {
            resolve_data: usvg::ImageHrefResolver::default_data_resolver(),
            resolve_string: Box::new(|_, _| None),
        },
        ..Default::default()
    };
    // Loading system fonts costs tens of milliseconds; only do it for SVGs with text.
    if data.windows(5).any(|w| w == b"<text") {
        let mut fonts = usvg::fontdb::Database::new();
        fonts.load_system_fonts();
        set_generic_families(&mut fonts);
        options.fontdb = Arc::new(fonts);
    }
    let tree = usvg::Tree::from_data(data, &options).map_err(|e| e.to_string())?;
    let size = tree.size();
    let (width, height) = (size.width(), size.height());
    check_size(width.ceil() as u32, height.ceil() as u32)?;
    Ok(Decoded::Svg {
        tree: Box::new(tree),
        width,
        height,
    })
}

/// Points the generic families (`serif`, `sans-serif`, `monospace`) at installed fonts.
/// fontdb defaults them to Times New Roman, Arial and Courier New; without those (most
/// Linux systems) text in `font-family="Times,serif"` (graphviz output) fell back to
/// an arbitrary font, a symbol font on some systems.
fn set_generic_families(fonts: &mut resvg::usvg::fontdb::Database) {
    const SERIF: &[&str] = &[
        "Times New Roman",
        "Liberation Serif",
        "DejaVu Serif",
        "Noto Serif",
        "Nimbus Roman",
        "FreeSerif",
    ];
    const SANS: &[&str] = &[
        "Arial",
        "Liberation Sans",
        "DejaVu Sans",
        "Noto Sans",
        "Nimbus Sans",
        "FreeSans",
    ];
    const MONO: &[&str] = &[
        "Courier New",
        "Liberation Mono",
        "DejaVu Sans Mono",
        "Noto Sans Mono",
        "Nimbus Mono PS",
        "FreeMono",
    ];
    let installed = |names: &[&str]| -> Option<String> {
        names
            .iter()
            .find(|name| {
                fonts
                    .faces()
                    .any(|face| face.families.iter().any(|(family, _)| family == *name))
            })
            .map(|name| name.to_string())
    };
    let (serif, sans, mono) = (installed(SERIF), installed(SANS), installed(MONO));
    if let Some(name) = serif {
        fonts.set_serif_family(name);
    }
    if let Some(name) = sans {
        fonts.set_sans_serif_family(name);
    }
    if let Some(name) = mono {
        fonts.set_monospace_family(name);
    }
}

/// Whether text looks like an SVG document: it starts with `<svg`, possibly after an
/// XML declaration, comments or a doctype.
pub fn looks_like_svg(head: &[u8]) -> bool {
    let text = String::from_utf8_lossy(&head[..head.len().min(4096)]);
    let mut rest = text.trim_start_matches('\u{feff}').trim_start();
    loop {
        if rest.starts_with("<svg") {
            return true;
        }
        let end = if rest.starts_with("<?") {
            rest.find("?>").map(|i| i + 2)
        } else if rest.starts_with("<!--") {
            rest.find("-->").map(|i| i + 3)
        } else if rest.starts_with("<!") {
            rest.find('>').map(|i| i + 1)
        } else {
            None
        };
        match end {
            Some(end) => rest = rest[end..].trim_start(),
            None => return false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn svg_sniffing() {
        assert!(looks_like_svg(b"<svg xmlns='http://www.w3.org/2000/svg'/>"));
        assert!(looks_like_svg(
            b"\xef\xbb\xbf<?xml version=\"1.0\"?>\n<!-- c -->\n<!DOCTYPE svg>\n<svg>"
        ));
        assert!(!looks_like_svg(b"<html><body><svg></svg></body></html>"));
        assert!(!looks_like_svg(b"const icon = '<svg>';"));
    }

    #[test]
    fn refuses_huge_dimensions_from_the_header() {
        // A valid PNG header claiming 20000x20000 pixels, without the pixel data.
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        png.extend_from_slice(&20000u32.to_be_bytes());
        png.extend_from_slice(&20000u32.to_be_bytes());
        png.extend_from_slice(&[8, 6, 0, 0, 0]);
        let crc = crc32(&png[12..]);
        png.extend_from_slice(&crc.to_be_bytes());
        // An empty IDAT chunk, so the header reader reaches the image data.
        png.extend_from_slice(b"\0\0\0\0IDAT");
        png.extend_from_slice(&crc32(b"IDAT").to_be_bytes());
        let err = Decoded::open(png, false).err().unwrap();
        assert!(err.contains("too large"), "{err}");
    }

    #[test]
    fn renders_svg_at_the_requested_size() {
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="20"><rect width="10" height="20" fill="red"/></svg>"#;
        let decoded = Decoded::open(svg.to_vec(), true).unwrap();
        assert_eq!(decoded.size(), (10, 20));
        let img = decoded.render_svg(40, 80).unwrap();
        assert_eq!(img.dimensions(), (40, 80));
        assert_eq!(img.get_pixel(20, 40).0, [255, 0, 0, 255]);
    }

    fn crc32(data: &[u8]) -> u32 {
        let mut crc = !0u32;
        for &b in data {
            crc ^= u32::from(b);
            for _ in 0..8 {
                crc = if crc & 1 != 0 {
                    (crc >> 1) ^ 0xedb8_8320
                } else {
                    crc >> 1
                };
            }
        }
        !crc
    }
}
