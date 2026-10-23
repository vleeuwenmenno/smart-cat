//! Image output: kitty graphics, sixel, Unicode blocks and ASCII.
//!
//! Entry points:
//! - [`Screen::detect`] decides once per run how images are drawn and how big they
//!   may be (method, terminal size, cell size, colours).
//! - [`show`] draws one decoded image with that screen. Other renderers (cover art,
//!   video poster frames, PDF pages, markdown images) use this.
//! - [`view`] is the file renderer: decode, play animations, print a caption.
//!
//! Nothing in this module runs for plain text files, so the decoders and resvg cost no
//! startup time there.

mod anim;
mod decode;
pub mod kitty;
pub mod sixel;
pub mod size;
pub mod text;

use std::io::{self, Read, Write};

use image::imageops::FilterType;
use image::{DynamicImage, RgbaImage};

use crate::config::Protocol;
use crate::input::Input;
use crate::term::{ColorDepth, query::Capabilities};

pub use decode::looks_like_svg;

/// Files larger than this are not read into memory for decoding.
const MAX_FILE_BYTES: u64 = 512 * 1024 * 1024;

/// How images are drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Kitty,
    Sixel,
    HalfBlocks,
    Sextants,
    Ascii,
}

impl Method {
    pub fn name(self) -> &'static str {
        match self {
            Method::Kitty => "kitty graphics protocol",
            Method::Sixel => "sixel",
            Method::HalfBlocks => "unicode half blocks",
            Method::Sextants => "unicode sextants",
            Method::Ascii => "ascii",
        }
    }

    fn is_pixel(self) -> bool {
        matches!(self, Method::Kitty | Method::Sixel)
    }
}

/// What [`Screen::detect`] needs to know about the run.
pub struct Env<'a> {
    /// `--protocol`, `SCAT_PROTOCOL` or the config value.
    pub protocol: Protocol,
    /// Where `protocol` came from, for `--diagnose` ("--protocol", "config").
    pub protocol_source: &'static str,
    /// Terminal query answers, if the terminal was queried and answered.
    pub caps: Option<&'a Capabilities>,
    pub stdout_tty: bool,
    pub utf8: bool,
    pub colors: bool,
    pub depth: ColorDepth,
    pub multiplexer: Option<&'static str>,
    /// `[image] max_height` in rows.
    pub max_height: Option<u16>,
}

/// Whether choosing a method for `protocol` needs the terminal query (it is skipped
/// for explicit text methods and when stdout is not a terminal).
pub fn needs_query(protocol: Protocol, stdout_tty: bool) -> bool {
    stdout_tty && matches!(protocol, Protocol::Auto | Protocol::Kitty | Protocol::Sixel)
}

/// The output method and the space images may use, decided once per run.
#[derive(Debug, Clone)]
pub struct Screen {
    pub method: Method,
    /// Why `method` was chosen, for `--diagnose`.
    pub reason: String,
    /// Terminal size in cells.
    pub cols: u16,
    pub rows: u16,
    /// Default maximum image height in rows.
    pub max_rows: u16,
    /// Cell size in pixels, and whether the terminal reported it.
    pub cell: (u16, u16),
    pub cell_known: bool,
    pub depth: ColorDepth,
    pub colors: bool,
    pub background: Option<(u8, u8, u8)>,
    /// Output is a terminal (animations play only then).
    pub tty: bool,
}

impl Screen {
    pub fn detect(env: &Env) -> Screen {
        let (method, reason) = choose(env);
        let (cols, rows) = size::terminal_cells();
        let reported = env
            .caps
            .and_then(|c| c.cell_pixels)
            .filter(|&(w, h)| w > 0 && h > 0);
        Screen {
            method,
            reason,
            cols,
            rows,
            max_rows: env
                .max_height
                .filter(|&h| h > 0)
                .unwrap_or_else(|| size::default_max_rows(rows)),
            cell: reported.unwrap_or(size::DEFAULT_CELL),
            cell_known: reported.is_some(),
            depth: env.depth,
            colors: env.colors,
            background: env.caps.and_then(|c| c.background),
            tty: env.stdout_tty,
        }
    }

    /// Cells and pixel grid for an image of `width`x`height` pixels.
    fn layout(&self, width: u32, height: u32, max_cols: u16, max_rows: u16) -> Layout {
        if self.method.is_pixel() {
            let fit = size::fit_pixels(width, height, max_cols, max_rows, self.cell, false);
            let mut px_h = fit.height;
            if self.method == Method::Sixel {
                // Whole sixel bands only, so the image ends exactly where we expect.
                px_h = (px_h / 6 * 6).max(6);
            }
            return Layout {
                cols: fit.cols,
                rows: fit.rows,
                px: (fit.width, px_h),
            };
        }
        let (cols, rows) = size::fit_cells(width, height, max_cols, max_rows, self.cell);
        let (sx, sy) = match self.method {
            Method::HalfBlocks => (1, 2),
            Method::Sextants => (2, 3),
            _ => (1, 1),
        };
        Layout {
            cols,
            rows,
            px: (u32::from(cols) * sx, u32::from(rows) * sy),
        }
    }

    fn limits(&self, opts: &ShowOptions) -> (u16, u16) {
        let cols = opts.max_cols.map_or(self.cols, |c| c.min(self.cols)).max(1);
        let rows = opts
            .max_rows
            .map_or(self.max_rows, |r| r.min(self.max_rows))
            .max(1);
        (cols, rows)
    }

    fn palette(&self) -> text::Palette {
        text::Palette {
            depth: self.depth,
            background: self.background.map_or([0, 0, 0], |(r, g, b)| [r, g, b]),
            colors: self.colors,
        }
    }

    fn light_background(&self) -> bool {
        self.background.is_some_and(|(r, g, b)| {
            0.2126 * f64::from(r) + 0.7152 * f64::from(g) + 0.0722 * f64::from(b) > 128.0
        })
    }
}

fn choose(env: &Env) -> (Method, String) {
    let text_fallback = |why: &str| -> (Method, String) {
        if !env.colors {
            (Method::Ascii, format!("{why}; colours are off"))
        } else if !env.utf8 {
            (Method::Ascii, format!("{why}; the locale is not UTF-8"))
        } else {
            (Method::HalfBlocks, format!("{why}; UTF-8 locale"))
        }
    };
    let src = env.protocol_source;
    match env.protocol {
        Protocol::Kitty => (Method::Kitty, format!("set by {src}")),
        Protocol::Sixel => (Method::Sixel, format!("set by {src}")),
        Protocol::Ascii => (Method::Ascii, format!("set by {src}")),
        Protocol::Blocks | Protocol::Sextants if !env.colors => (
            Method::Ascii,
            format!("{src} asks for {}, but colours are off", env.protocol),
        ),
        Protocol::Blocks => (Method::HalfBlocks, format!("set by {src}")),
        Protocol::Sextants => (Method::Sextants, format!("set by {src}")),
        Protocol::Auto if !env.stdout_tty => {
            text_fallback("output is not a terminal, so no queries")
        }
        Protocol::Auto => {
            let tmux = env.multiplexer == Some("tmux");
            match env.caps {
                Some(c) if c.kitty_graphics && !tmux => (
                    Method::Kitty,
                    "the terminal answered the kitty graphics query".into(),
                ),
                Some(c) if c.sixel => (
                    Method::Sixel,
                    if tmux {
                        "tmux reported sixel support (DA1 attribute 4)".into()
                    } else {
                        "the terminal reported sixel support (DA1 attribute 4)".into()
                    },
                ),
                Some(c) if c.kitty_graphics => text_fallback(
                    "kitty graphics answered through tmux, but tmux does not pass images on",
                ),
                Some(_) => text_fallback("no graphics protocol confirmed by the terminal"),
                None => text_fallback("the terminal did not answer the query"),
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Layout {
    cols: u16,
    rows: u16,
    /// Pixel size the image is resized to before encoding.
    px: (u32, u32),
}

/// Limits and decorations for [`show`].
#[derive(Debug, Clone, Default)]
pub struct ShowOptions<'a> {
    /// At most this many columns (default and upper bound: the terminal width).
    pub max_cols: Option<u16>,
    /// At most this many rows (default and upper bound: `[image] max_height` or the
    /// terminal height minus 4).
    pub max_rows: Option<u16>,
    /// A dim line printed under the image. Sanitized before printing.
    pub caption: Option<&'a str>,
}

/// Draws `img` with the method and size limits of `screen`, then the caption.
///
/// The image is scaled to fit `max_cols` x `max_rows` with its aspect ratio kept.
/// kitty and sixel never enlarge an image beyond its pixel size; text methods may.
/// The cursor ends at the start of the line below the image (and caption). Only the
/// encoded image data and scat's own control sequences are written.
///
/// ```ignore
/// let screen = ctx.image_screen();
/// image::show(out, &screen, &cover, &ShowOptions { max_rows: Some(12), caption: Some("cover"), ..Default::default() })?;
/// ```
#[allow(dead_code)] // for cover art, poster frames, PDF pages and markdown images
pub fn show<W: Write>(
    out: &mut W,
    screen: &Screen,
    img: &DynamicImage,
    opts: &ShowOptions,
) -> io::Result<()> {
    let (max_cols, max_rows) = screen.limits(opts);
    let layout = screen.layout(img.width(), img.height(), max_cols, max_rows);
    let frame = encode(screen, &layout, img, None, FrameMode::Still)?;
    out.write_all(&frame)?;
    finish_still(out, screen)?;
    write_caption(out, screen, opts.caption)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FrameMode {
    Still,
    /// A frame of an animation drawn in place: opaque, and kitty keeps the cursor.
    Animated {
        image_id: u32,
    },
}

/// Encodes one frame, resized to the layout. `png` is the original file when it is a
/// PNG, so kitty can take it without re-encoding when no resizing is needed.
fn encode(
    screen: &Screen,
    layout: &Layout,
    img: &DynamicImage,
    png: Option<&[u8]>,
    mode: FrameMode,
) -> io::Result<Vec<u8>> {
    let mut buf = Vec::new();
    let (w, h) = layout.px;
    match screen.method {
        Method::Kitty => {
            let placement = kitty::Placement {
                cols: layout.cols,
                ids: match mode {
                    FrameMode::Animated { image_id } => Some((image_id, 1)),
                    FrameMode::Still => None,
                },
                keep_cursor: mode != FrameMode::Still,
            };
            match png {
                Some(png) if (img.width(), img.height()) == (w, h) => {
                    kitty::write_png(&mut buf, png, placement)?
                }
                _ => {
                    let rgba = resize(img, w, h);
                    let mut encoded = Vec::new();
                    image::codecs::png::PngEncoder::new_with_quality(
                        &mut encoded,
                        image::codecs::png::CompressionType::Fast,
                        image::codecs::png::FilterType::Adaptive,
                    )
                    .write_image(&rgba, w, h, image::ExtendedColorType::Rgba8)
                    .map_err(io::Error::other)?;
                    kitty::write_png(&mut buf, &encoded, placement)?;
                }
            }
        }
        Method::Sixel => {
            let mut rgba = resize(img, w, h);
            if mode != FrameMode::Still {
                // Unpainted pixels would show the previous frame.
                flatten(&mut rgba, screen.palette().background);
            }
            sixel::write_sixel(&mut buf, &rgba)?;
        }
        Method::HalfBlocks => {
            text::write_half_blocks(&mut buf, &resize(img, w, h), &screen.palette())?
        }
        Method::Sextants => text::write_sextants(&mut buf, &resize(img, w, h), &screen.palette())?,
        Method::Ascii => text::write_ascii(
            &mut buf,
            &resize(img, w, h),
            &screen.palette(),
            screen.light_background(),
        )?,
    }
    Ok(buf)
}

use image::ImageEncoder as _;

/// Moves the cursor below a still image.
fn finish_still<W: Write>(out: &mut W, screen: &Screen) -> io::Result<()> {
    match screen.method {
        // kitty leaves the cursor after the last column of the image's last row; sixel
        // terminals leave it on the image's last text row.
        Method::Kitty | Method::Sixel => out.write_all(b"\n"),
        // Text lines already end with a newline.
        _ => Ok(()),
    }
}

fn write_caption<W: Write>(out: &mut W, screen: &Screen, caption: Option<&str>) -> io::Result<()> {
    let Some(caption) = caption else {
        return Ok(());
    };
    let caption = crate::render::sanitize_str(caption);
    if screen.colors {
        writeln!(out, "\x1b[2m{caption}\x1b[22m")
    } else {
        writeln!(out, "{caption}")
    }
}

/// Resizes to exactly `width`x`height`. Large reductions first use a fast area
/// reduction, then a Catmull-Rom pass.
fn resize(img: &DynamicImage, width: u32, height: u32) -> RgbaImage {
    let (width, height) = (width.max(1), height.max(1));
    if (img.width(), img.height()) == (width, height) {
        return img.to_rgba8();
    }
    if img.width() >= width * 4 && img.height() >= height * 4 {
        let rough = img.thumbnail_exact(width * 2, height * 2);
        return image::imageops::resize(&rough.to_rgba8(), width, height, FilterType::Triangle);
    }
    image::imageops::resize(&img.to_rgba8(), width, height, FilterType::CatmullRom)
}

fn flatten(img: &mut RgbaImage, bg: [u8; 3]) {
    for p in img.pixels_mut() {
        let a = u16::from(p[3]);
        for k in 0..3 {
            p[k] = ((u16::from(p[k]) * a + u16::from(bg[k]) * (255 - a)) / 255) as u8;
        }
        p[3] = 255;
    }
}

/// Options for [`view`].
pub struct ViewOptions<'a> {
    /// Shown in the caption and messages.
    pub name: &'a str,
    pub file_size: Option<u64>,
    /// Treat the input as SVG.
    pub svg: bool,
    /// Play animations (terminal output only).
    pub animate: bool,
    pub caption: bool,
}

/// Renders an image file: decode, draw (or play once), caption. Decoding problems
/// become an error `cannot decode image: ...` for the caller to report.
pub fn view<W: Write>(
    input: &mut Input,
    out: &mut W,
    screen: &Screen,
    opts: &ViewOptions,
) -> io::Result<()> {
    let mut data = Vec::new();
    (&mut input.reader)
        .take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut data)?;
    if data.len() as u64 > MAX_FILE_BYTES {
        return Err(io::Error::other(format!(
            "image file larger than {} MiB, not decoded",
            MAX_FILE_BYTES >> 20
        )));
    }
    let size = opts.file_size.unwrap_or(data.len() as u64);
    let decoded = decode::Decoded::open(data, opts.svg).map_err(cannot_decode)?;
    let (width, height) = decoded.size();
    let (max_cols, max_rows) = screen.limits(&ShowOptions::default());
    let mut frames_shown = 1;

    if let decode::Decoded::Svg { .. } = decoded {
        // Vectors stay sharp when enlarged; allow up to 4x the intrinsic size.
        let layout = screen.layout(
            width.saturating_mul(4),
            height.saturating_mul(4),
            max_cols,
            max_rows,
        );
        let img = DynamicImage::ImageRgba8(
            decoded
                .render_svg(layout.px.0, layout.px.1)
                .map_err(cannot_decode)?,
        );
        out.write_all(&encode(screen, &layout, &img, None, FrameMode::Still)?)?;
        finish_still(out, screen)?;
    } else {
        let layout = screen.layout(width, height, max_cols, max_rows);
        let played = if opts.animate && screen.tty {
            anim::play(out, screen, &layout, &decoded)?
        } else {
            None
        };
        match played {
            Some(count) => frames_shown = count,
            None => {
                let img = decoded.first_frame().map_err(cannot_decode)?;
                out.write_all(&encode(
                    screen,
                    &layout,
                    &img,
                    decoded.png_bytes(),
                    FrameMode::Still,
                )?)?;
                finish_still(out, screen)?;
            }
        }
    }

    if opts.caption {
        let frames = if frames_shown > 1 {
            format!(", {frames_shown} frames")
        } else {
            String::new()
        };
        let caption = format!(
            "{}  {width}x{height} {}{frames}, {}",
            opts.name,
            decoded.format_name(),
            crate::util::human_size(size)
        );
        write_caption(out, screen, Some(&caption))?;
    }
    Ok(())
}

fn cannot_decode(err: String) -> io::Error {
    io::Error::other(format!("cannot decode image: {err}"))
}

/// Whether a file should go to the image view when its renderer is `auto`: a
/// supported raster type by content, or an SVG by extension or content.
pub fn wants(mime: &str, file_name: Option<&str>, head: &[u8]) -> bool {
    const RASTER: &[&str] = &[
        "image/png",
        "image/jpeg",
        "image/gif",
        "image/webp",
        "image/bmp",
        "image/tiff",
        "image/vnd.microsoft.icon",
        "image/x-icon",
    ];
    if RASTER.contains(&mime) {
        return true;
    }
    let ext = file_name
        .and_then(|n| n.rsplit_once('.'))
        .map(|(_, e)| e.to_ascii_lowercase());
    match ext.as_deref() {
        Some("svg") => true,
        Some("qoi" | "tga" | "pnm" | "pbm" | "pgm" | "ppm" | "pam") => !head.is_empty(),
        _ => is_svg(file_name, head),
    }
}

/// Whether the content should be parsed as SVG.
pub fn is_svg(file_name: Option<&str>, head: &[u8]) -> bool {
    file_name.is_some_and(|n| n.to_ascii_lowercase().ends_with(".svg")) || looks_like_svg(head)
}
