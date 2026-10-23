//! Text-cell image output: half blocks, sextants and an ASCII ramp.
//!
//! The caller resizes the image to the cell grid times the sub-pixels per cell:
//! 1x2 for half blocks, 2x3 for sextants, 1x1 for ASCII.

use std::io::{self, Write};

use image::RgbaImage;

use crate::highlight::{rgb_to_16, rgb_to_256};
use crate::term::ColorDepth;

/// Pixels with less alpha than this count as transparent (terminal background).
const ALPHA_CUTOFF: u8 = 128;
const RAMP: &[u8] = b" .:-=+*#%@";

type Rgb = [u8; 3];

/// Colours for text output.
#[derive(Debug, Clone, Copy)]
pub struct Palette {
    pub depth: ColorDepth,
    /// Background that partly transparent pixels are blended onto, when the terminal
    /// reported it. Without it, such pixels keep their own colour: blending onto a
    /// guessed black looks wrong on light terminals.
    pub background: Option<Rgb>,
    /// Emit SGR colours at all (ASCII without colour is a plain ramp).
    pub colors: bool,
}

impl Palette {
    fn blend(&self, p: &image::Rgba<u8>) -> Option<Rgb> {
        if p[3] < ALPHA_CUTOFF {
            return None;
        }
        let Some(bg) = self.background else {
            return Some([p[0], p[1], p[2]]);
        };
        let a = u16::from(p[3]);
        let mix = |c: u8, bg: u8| ((u16::from(c) * a + u16::from(bg) * (255 - a)) / 255) as u8;
        Some([mix(p[0], bg[0]), mix(p[1], bg[1]), mix(p[2], bg[2])])
    }
}

/// SGR parameters for a foreground (`bg == false`) or background colour.
pub fn color_params(rgb: Rgb, depth: ColorDepth, bg: bool) -> String {
    let [r, g, b] = rgb;
    match depth {
        ColorDepth::TrueColor => format!("{};2;{r};{g};{b}", if bg { 48 } else { 38 }),
        ColorDepth::Ansi256 => format!("{};5;{}", if bg { 48 } else { 38 }, rgb_to_256(r, g, b)),
        ColorDepth::Ansi16 => {
            let idx = rgb_to_16(r, g, b);
            let base = match (bg, idx < 8) {
                (false, true) => 30,
                (false, false) => 90 - 8,
                (true, true) => 40,
                (true, false) => 100 - 8,
            };
            format!("{}", base + u16::from(idx))
        }
    }
}

/// Writes cells, emitting SGR only when the colours change. `None` is the terminal's
/// default colour, which is also the state after the reset at each line end.
struct CellWriter<'a, W: Write> {
    out: &'a mut W,
    depth: ColorDepth,
    fg: Option<Rgb>,
    bg: Option<Rgb>,
}

impl<'a, W: Write> CellWriter<'a, W> {
    fn new(out: &'a mut W, depth: ColorDepth) -> Self {
        Self {
            out,
            depth,
            fg: None,
            bg: None,
        }
    }

    fn cell(&mut self, fg: Option<Rgb>, bg: Option<Rgb>, ch: &str) -> io::Result<()> {
        let mut params = Vec::new();
        if self.fg != fg {
            params.push(fg.map_or("39".into(), |c| color_params(c, self.depth, false)));
            self.fg = fg;
        }
        if self.bg != bg {
            params.push(bg.map_or("49".into(), |c| color_params(c, self.depth, true)));
            self.bg = bg;
        }
        if !params.is_empty() {
            write!(self.out, "\x1b[{}m", params.join(";"))?;
        }
        self.out.write_all(ch.as_bytes())
    }

    fn end_line(&mut self) -> io::Result<()> {
        let reset = self.fg.is_some() || self.bg.is_some();
        self.fg = None;
        self.bg = None;
        self.out.write_all(if reset { b"\x1b[0m\n" } else { b"\n" })
    }
}

/// Half blocks: each cell shows two vertically stacked pixels, top as the `▀`
/// foreground, bottom as the background. `img` is cols x (2 * rows).
pub fn write_half_blocks<W: Write>(out: &mut W, img: &RgbaImage, pal: &Palette) -> io::Result<()> {
    let mut w = CellWriter::new(out, pal.depth);
    for row in 0..img.height().div_ceil(2) {
        for x in 0..img.width() {
            let top = pal.blend(img.get_pixel(x, row * 2));
            let bottom = if row * 2 + 1 < img.height() {
                pal.blend(img.get_pixel(x, row * 2 + 1))
            } else {
                None
            };
            let (fg, bg, ch) = half_block_cell(top, bottom);
            w.cell(fg, bg, ch)?;
        }
        w.end_line()?;
    }
    Ok(())
}

/// Chooses (foreground, background, character) for a top and bottom pixel; `None`
/// is the terminal's default colour.
fn half_block_cell(
    top: Option<Rgb>,
    bottom: Option<Rgb>,
) -> (Option<Rgb>, Option<Rgb>, &'static str) {
    match (top, bottom) {
        (None, None) => (None, None, " "),
        (Some(t), None) => (Some(t), None, "▀"),
        (None, Some(b)) => (Some(b), None, "▄"),
        (Some(t), Some(b)) => (Some(t), Some(b), "▀"),
    }
}

/// Sextants: 2x3 pixels per cell, two colours per cell. `img` is (2 * cols) x (3 * rows).
pub fn write_sextants<W: Write>(out: &mut W, img: &RgbaImage, pal: &Palette) -> io::Result<()> {
    let mut w = CellWriter::new(out, pal.depth);
    let mut buf = [0u8; 4];
    for row in 0..img.height().div_ceil(3) {
        for col in 0..img.width().div_ceil(2) {
            let mut px = [None; 6];
            for (i, p) in px.iter_mut().enumerate() {
                let (x, y) = (col * 2 + (i as u32 % 2), row * 3 + (i as u32 / 2));
                if x < img.width() && y < img.height() {
                    *p = pal.blend(img.get_pixel(x, y));
                }
            }
            let (fg, bg, mask) = split_two_colors(&px);
            w.cell(fg, bg, sextant_char(mask).encode_utf8(&mut buf))?;
        }
        w.end_line()?;
    }
    Ok(())
}

/// Splits up to 6 pixels into a foreground and background group. Bit i of the mask is
/// set when pixel i (row-major, 2 per row) belongs to the foreground.
fn split_two_colors(px: &[Option<Rgb>; 6]) -> (Option<Rgb>, Option<Rgb>, u8) {
    let opaque: Vec<Rgb> = px.iter().flatten().copied().collect();
    if opaque.is_empty() {
        return (None, None, 0);
    }
    let dist = |a: Rgb, b: Rgb| {
        (0..3)
            .map(|k| (i32::from(a[k]) - i32::from(b[k])).pow(2))
            .sum::<i32>()
    };
    // Seeds: the two most different opaque pixels.
    let (mut a, mut b) = (opaque[0], opaque[0]);
    let mut far = -1;
    for &p in &opaque {
        for &q in &opaque {
            if dist(p, q) > far {
                far = dist(p, q);
                (a, b) = (p, q);
            }
        }
    }
    let mut mask = 0u8;
    let (mut sum_a, mut sum_b) = ([0u32; 3], [0u32; 3]);
    let (mut n_a, mut n_b) = (0u32, 0u32);
    let has_transparent = px.iter().any(Option::is_none);
    for (i, p) in px.iter().enumerate() {
        let Some(p) = *p else { continue };
        // With transparent pixels the background must stay the terminal default, so
        // every opaque pixel joins the foreground.
        if has_transparent || dist(p, a) <= dist(p, b) {
            mask |= 1 << i;
            n_a += 1;
            (0..3).for_each(|k| sum_a[k] += u32::from(p[k]));
        } else {
            n_b += 1;
            (0..3).for_each(|k| sum_b[k] += u32::from(p[k]));
        }
    }
    let avg = |s: [u32; 3], n: u32| (n > 0).then(|| s.map(|v| (v / n) as u8));
    let fg = avg(sum_a, n_a);
    let bg = if has_transparent {
        None
    } else {
        avg(sum_b, n_b)
    };
    (fg, bg.or(fg), mask)
}

/// The character for a 2x3 mask (bit 0 top-left, bit 1 top-right, ... bit 5
/// bottom-right).
fn sextant_char(mask: u8) -> char {
    match mask & 63 {
        0 => ' ',
        21 => '▌',
        42 => '▐',
        63 => '█',
        n => {
            let skip = u32::from(n > 21) + u32::from(n > 42);
            char::from_u32(0x1FB00 + u32::from(n) - 1 - skip).unwrap_or('?')
        }
    }
}

/// ASCII ramp: one pixel per cell, denser characters for brighter pixels on a dark
/// background (inverted on a light one). `img` is cols x rows.
pub fn write_ascii<W: Write>(
    out: &mut W,
    img: &RgbaImage,
    pal: &Palette,
    light_bg: bool,
) -> io::Result<()> {
    let mut w = CellWriter::new(out, pal.depth);
    let mut line = Vec::with_capacity(img.width() as usize);
    for y in 0..img.height() {
        line.clear();
        for x in 0..img.width() {
            let Some(rgb) = pal.blend(img.get_pixel(x, y)) else {
                if pal.colors {
                    w.cell(None, None, " ")?;
                } else {
                    line.push(b' ');
                }
                continue;
            };
            let ch = ramp_char(rgb, light_bg);
            if pal.colors {
                w.cell(Some(rgb), None, std::str::from_utf8(&[ch]).unwrap_or(" "))?;
            } else {
                line.push(ch);
            }
        }
        if pal.colors {
            w.end_line()?;
        } else {
            // Trailing spaces carry no information.
            while line.last() == Some(&b' ') {
                line.pop();
            }
            line.push(b'\n');
            w.out.write_all(&line)?;
        }
    }
    Ok(())
}

fn ramp_char(rgb: Rgb, light_bg: bool) -> u8 {
    let luma = (299 * u32::from(rgb[0]) + 587 * u32::from(rgb[1]) + 114 * u32::from(rgb[2])) / 1000;
    let luma = if light_bg { 255 - luma } else { luma };
    RAMP[(luma as usize * (RAMP.len() - 1) + 127) / 255]
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    fn pal(colors: bool) -> Palette {
        Palette {
            depth: ColorDepth::TrueColor,
            background: Some([0, 0, 0]),
            colors,
        }
    }

    #[test]
    fn half_block_composes_top_and_bottom() {
        let img = RgbaImage::from_fn(1, 2, |_, y| {
            if y == 0 {
                Rgba([255, 0, 0, 255])
            } else {
                Rgba([0, 0, 255, 255])
            }
        });
        let mut out = Vec::new();
        write_half_blocks(&mut out, &img, &pal(true)).unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "\x1b[38;2;255;0;0;48;2;0;0;255m▀\x1b[0m\n"
        );
    }

    #[test]
    fn half_block_transparency_uses_default_background() {
        assert_eq!(half_block_cell(None, None), (None, None, " "));
        assert_eq!(
            half_block_cell(Some([1, 2, 3]), None),
            (Some([1, 2, 3]), None, "▀")
        );
        assert_eq!(
            half_block_cell(None, Some([1, 2, 3])),
            (Some([1, 2, 3]), None, "▄")
        );
    }

    #[test]
    fn partial_alpha_blends_onto_the_reported_background_only() {
        let px = Rgba([200, 100, 0, 128]);
        let white = Palette {
            background: Some([255, 255, 255]),
            ..pal(true)
        };
        assert_eq!(white.blend(&px), Some([227, 177, 127]));
        let unknown = Palette {
            background: None,
            ..pal(true)
        };
        assert_eq!(unknown.blend(&px), Some([200, 100, 0]));
        assert_eq!(unknown.blend(&Rgba([1, 2, 3, 10])), None);
    }

    #[test]
    fn repeated_colours_are_not_repeated() {
        let img = RgbaImage::from_pixel(3, 2, Rgba([10, 20, 30, 255]));
        let mut out = Vec::new();
        write_half_blocks(&mut out, &img, &pal(true)).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert_eq!(text.matches("\x1b[38").count(), 1);
        assert!(text.contains("▀▀▀"));
    }

    #[test]
    fn colour_depths() {
        assert_eq!(
            color_params([255, 0, 0], ColorDepth::TrueColor, false),
            "38;2;255;0;0"
        );
        assert_eq!(
            color_params([255, 0, 0], ColorDepth::Ansi256, true),
            "48;5;196"
        );
        assert_eq!(color_params([250, 10, 10], ColorDepth::Ansi16, false), "91");
        assert_eq!(color_params([0, 0, 0], ColorDepth::Ansi16, true), "40");
    }

    #[test]
    fn ascii_ramp() {
        assert_eq!(ramp_char([0, 0, 0], false), b' ');
        assert_eq!(ramp_char([255, 255, 255], false), b'@');
        assert_eq!(ramp_char([255, 255, 255], true), b' ');
        let img = RgbaImage::from_fn(3, 1, |x, _| {
            let v = (x * 127) as u8;
            Rgba([v, v, v, 255])
        });
        let mut out = Vec::new();
        write_ascii(&mut out, &img, &pal(false), false).unwrap();
        assert_eq!(out, b" =@\n");
    }

    #[test]
    fn sextant_characters() {
        assert_eq!(sextant_char(0), ' ');
        assert_eq!(sextant_char(1), '\u{1FB00}');
        assert_eq!(sextant_char(20), '\u{1FB13}');
        assert_eq!(sextant_char(22), '\u{1FB14}');
        assert_eq!(sextant_char(62), '\u{1FB3B}');
        assert_eq!(sextant_char(21), '▌');
        assert_eq!(sextant_char(63), '█');
    }

    #[test]
    fn sextant_split_separates_colours() {
        let r = Some([255, 0, 0]);
        let b = Some([0, 0, 255]);
        let (fg, bg, mask) = split_two_colors(&[r, b, r, b, r, b]);
        assert_eq!(mask, 21);
        assert_eq!((fg, bg), (r, b));
    }
}
