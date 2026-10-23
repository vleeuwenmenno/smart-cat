//! Sixel output: median-cut quantisation to at most 256 colours, Floyd–Steinberg
//! dithering and a run-length encoded sixel stream.
//!
//! Transparent pixels (alpha < 128) are left unpainted (`P2=1`), so the terminal
//! background shows through.

use std::io::{self, Write};

use image::RgbaImage;

const MAX_COLORS: usize = 256;
/// Pixels sampled for building the palette.
const MAX_SAMPLES: usize = 1 << 16;
const TRANSPARENT: u16 = u16::MAX;

/// Encodes `img` as one sixel image. The caller should make the height a multiple
/// of 6 so the image ends exactly on a sixel band.
pub fn write_sixel<W: Write>(out: &mut W, img: &RgbaImage) -> io::Result<()> {
    let (width, height) = (img.width() as usize, img.height() as usize);
    let palette = median_cut(img, MAX_COLORS);
    let indices = dither(img, &palette);

    let mut buf = Vec::with_capacity(width * height / 2 + 4096);
    // P1=0 (aspect 2:1, overridden by raster attributes), P2=1 (transparent zeros).
    buf.extend_from_slice(b"\x1bP0;1;0q");
    write!(buf, "\"1;1;{width};{height}")?;
    for (i, c) in palette.iter().enumerate() {
        let pct = |v: u8| (u32::from(v) * 100 + 127) / 255;
        write!(buf, "#{i};2;{};{};{}", pct(c[0]), pct(c[1]), pct(c[2]))?;
    }

    let mut masks = vec![0u8; palette.len() * width];
    let mut used = vec![false; palette.len()];
    let mut order = Vec::with_capacity(palette.len());
    for band in 0..height.div_ceil(6) {
        masks.fill(0);
        used.fill(false);
        order.clear();
        for dy in 0..6 {
            let y = band * 6 + dy;
            if y >= height {
                break;
            }
            for x in 0..width {
                let idx = indices[y * width + x];
                if idx == TRANSPARENT {
                    continue;
                }
                let idx = usize::from(idx);
                if !used[idx] {
                    used[idx] = true;
                    order.push(idx);
                }
                masks[idx * width + x] |= 1 << dy;
            }
        }
        for (n, &color) in order.iter().enumerate() {
            if n > 0 {
                buf.push(b'$');
            }
            write!(buf, "#{color}")?;
            let row = &masks[color * width..(color + 1) * width];
            // Trailing empty columns need not be sent.
            let end = row.iter().rposition(|&m| m != 0).map_or(0, |p| p + 1);
            write_runs(&mut buf, &row[..end]);
        }
        buf.push(b'-');
    }
    buf.extend_from_slice(b"\x1b\\");
    out.write_all(&buf)
}

fn write_runs(buf: &mut Vec<u8>, row: &[u8]) {
    let mut i = 0;
    while i < row.len() {
        let mask = row[i];
        let mut run = 1;
        while i + run < row.len() && row[i + run] == mask {
            run += 1;
        }
        let ch = b'?' + mask;
        if run > 3 {
            let _ = write!(buf, "!{run}");
            buf.push(ch);
        } else {
            buf.extend(std::iter::repeat_n(ch, run));
        }
        i += run;
    }
}

/// Builds a palette of at most `max` colours from the opaque pixels.
fn median_cut(img: &RgbaImage, max: usize) -> Vec<[u8; 3]> {
    let total = (img.width() as usize) * (img.height() as usize);
    let step = total.div_ceil(MAX_SAMPLES).max(1);
    let mut samples: Vec<[u8; 3]> = img
        .pixels()
        .step_by(step)
        .filter(|p| p[3] >= 128)
        .map(|p| [p[0], p[1], p[2]])
        .collect();
    if samples.is_empty() {
        return vec![[0, 0, 0]];
    }
    // Boxes are ranges of `samples`; split the box with the widest channel range.
    let mut boxes: Vec<(usize, usize)> = vec![(0, samples.len())];
    while boxes.len() < max {
        let mut best = None;
        let mut best_range = 0u8;
        for (b, &(start, end)) in boxes.iter().enumerate() {
            if end - start < 2 {
                continue;
            }
            let (channel, range) = widest_channel(&samples[start..end]);
            if range > best_range {
                best_range = range;
                best = Some((b, channel));
            }
        }
        let Some((b, channel)) = best else { break };
        let (start, end) = boxes[b];
        let slice = &mut samples[start..end];
        let mid = slice.len() / 2;
        slice.select_nth_unstable_by_key(mid, |c| c[channel]);
        boxes[b] = (start, start + mid);
        boxes.push((start + mid, end));
    }
    boxes
        .iter()
        .map(|&(start, end)| {
            let mut sum = [0u64; 3];
            for c in &samples[start..end] {
                for k in 0..3 {
                    sum[k] += u64::from(c[k]);
                }
            }
            let n = (end - start).max(1) as u64;
            [(sum[0] / n) as u8, (sum[1] / n) as u8, (sum[2] / n) as u8]
        })
        .collect()
}

fn widest_channel(colors: &[[u8; 3]]) -> (usize, u8) {
    let mut lo = [255u8; 3];
    let mut hi = [0u8; 3];
    for c in colors {
        for k in 0..3 {
            lo[k] = lo[k].min(c[k]);
            hi[k] = hi[k].max(c[k]);
        }
    }
    (0..3)
        .map(|k| (k, hi[k] - lo[k]))
        .max_by_key(|&(_, r)| r)
        .unwrap_or((0, 0))
}

/// Maps every pixel to a palette index with Floyd–Steinberg error diffusion.
/// Transparent pixels become `TRANSPARENT`.
fn dither(img: &RgbaImage, palette: &[[u8; 3]]) -> Vec<u16> {
    let (width, height) = (img.width() as usize, img.height() as usize);
    // Nearest-colour cache on 5 bits per channel.
    let mut cache = vec![u16::MAX; 1 << 15];
    let mut nearest = |r: u8, g: u8, b: u8| -> u16 {
        let key = (usize::from(r >> 3) << 10) | (usize::from(g >> 3) << 5) | usize::from(b >> 3);
        if cache[key] == u16::MAX {
            let (r, g, b) = (i32::from(r), i32::from(g), i32::from(b));
            let mut best = 0;
            let mut best_d = i32::MAX;
            for (i, c) in palette.iter().enumerate() {
                let d = (r - i32::from(c[0])).pow(2)
                    + (g - i32::from(c[1])).pow(2)
                    + (b - i32::from(c[2])).pow(2);
                if d < best_d {
                    best_d = d;
                    best = i;
                }
            }
            cache[key] = best as u16;
        }
        cache[key]
    };

    let mut out = vec![TRANSPARENT; width * height];
    // Error rows for the current and next line, 3 channels each, with a 1-pixel margin.
    let mut cur = vec![[0i16; 3]; width + 2];
    let mut next = vec![[0i16; 3]; width + 2];
    for y in 0..height {
        for x in 0..width {
            let p = img.get_pixel(x as u32, y as u32);
            if p[3] < 128 {
                continue;
            }
            let e = cur[x + 1];
            let want = [
                (i16::from(p[0]) + e[0]).clamp(0, 255),
                (i16::from(p[1]) + e[1]).clamp(0, 255),
                (i16::from(p[2]) + e[2]).clamp(0, 255),
            ];
            let idx = nearest(want[0] as u8, want[1] as u8, want[2] as u8);
            out[y * width + x] = idx;
            let got = palette[usize::from(idx)];
            for k in 0..3 {
                let err = want[k] - i16::from(got[k]);
                cur[x + 2][k] += err * 7 / 16;
                next[x][k] += err * 3 / 16;
                next[x + 1][k] += err * 5 / 16;
                next[x + 2][k] += err / 16;
            }
        }
        std::mem::swap(&mut cur, &mut next);
        next.iter_mut().for_each(|e| *e = [0; 3]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    fn encode(img: &RgbaImage) -> Vec<u8> {
        let mut out = Vec::new();
        write_sixel(&mut out, img).unwrap();
        out
    }

    #[test]
    fn framing_and_raster_attributes() {
        let img = RgbaImage::from_fn(10, 12, |x, _| {
            if x < 5 {
                Rgba([255, 0, 0, 255])
            } else {
                Rgba([0, 0, 255, 255])
            }
        });
        let out = encode(&img);
        assert!(out.starts_with(b"\x1bP0;1;0q\"1;1;10;12"));
        assert!(out.ends_with(b"-\x1b\\"));
        // Two bands of 6 rows.
        assert_eq!(out.iter().filter(|&&b| b == b'-').count(), 2);
        // The only escape bytes are the DCS introducer and the string terminator.
        assert_eq!(out.iter().filter(|&&b| b == 0x1b).count(), 2);
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains(";2;100;0;0"), "red in palette: {text}");
        assert!(text.contains(";2;0;0;100"), "blue in palette: {text}");
        // Runs of 5 identical full columns are compressed.
        assert!(text.contains("!5~"), "{text}");
    }

    #[test]
    fn transparent_pixels_are_not_painted() {
        let img = RgbaImage::from_pixel(4, 6, Rgba([0, 0, 0, 0]));
        let text = String::from_utf8(encode(&img)).unwrap();
        assert!(text.ends_with("#0;2;0;0;0-\x1b\\"), "{text:?}");
    }

    #[test]
    fn palette_never_exceeds_256() {
        let img = RgbaImage::from_fn(64, 64, |x, y| {
            Rgba([(x * 4) as u8, (y * 4) as u8, 128, 255])
        });
        assert!(median_cut(&img, MAX_COLORS).len() <= 256);
        let indices = dither(&img, &median_cut(&img, MAX_COLORS));
        assert!(indices.iter().all(|&i| i < 256));
    }

    #[test]
    fn run_length_encoding() {
        let mut buf = Vec::new();
        write_runs(&mut buf, &[63, 63, 63, 63, 63, 1, 1, 0]);
        assert_eq!(buf, b"!5~@@?");
    }
}
