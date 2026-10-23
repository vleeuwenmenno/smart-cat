//! The specimen image: sample lines set in the font, drawn with resvg.
//!
//! The lines become an SVG document with `<text>` elements. usvg shapes them with
//! harfrust (kerning, ligatures, complex scripts) and converts the glyphs to paths;
//! resvg fills them. The font database holds only the face being shown and the font
//! resolver always returns it, so no system font is loaded or substituted.
//!
//! Dark text on a light "paper" card, whatever the terminal background: the specimen
//! looks the same everywhere and needs no terminal query.

use std::sync::Arc;

use image::RgbaImage;
use resvg::usvg;
use skrifa::{FontRef, MetadataProvider};

use super::info::{Face, LATIN};

const PAPER: (u8, u8, u8) = (0xf7, 0xf6, 0xf2);
const INK: &str = "#1b1b1b";
const SOFT_INK: &str = "#6b6b6b";
/// Margin around the text, in SVG pixels.
const PAD: f32 = 24.0;
/// Widest specimen before scaling, in SVG pixels; longer lines are cut off.
const MAX_WIDTH: f32 = 2400.0;

struct Line {
    text: String,
    size: f32,
    /// `font-variation-settings`, when it differs from the default instance.
    variation: Option<String>,
    soft: bool,
}

impl Line {
    fn new(text: impl Into<String>, size: f32) -> Line {
        Line {
            text: text.into(),
            size,
            variation: None,
            soft: false,
        }
    }
}

const ALPHABET: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZ abcdefghijklmnopqrstuvwxyz 0123456789 !@#$%&*()";
const CODE: [&str; 3] = [
    "fn main() { let x = [1, 2, 3]; }",
    "if (a != b && c <= d) { return i++ * 0x1F; }",
    "0O 1lI| {}[]() => -> == != <= >= // /* */",
];

/// Whether every non-space character of `text` has a glyph.
fn covers(font: &FontRef, text: &str) -> bool {
    let charmap = font.charmap();
    text.chars()
        .filter(|c| !c.is_whitespace())
        .all(|c| charmap.map(c).is_some())
}

fn lines(font: &FontRef, face: &Face) -> Vec<Line> {
    let mut lines = Vec::new();
    let title = if face.full_name.is_empty() {
        &face.family
    } else {
        &face.full_name
    };
    let title: String = title.chars().filter(|c| !c.is_control()).take(80).collect();
    if !title.is_empty() && covers(font, &title) {
        lines.push(Line::new(title, 40.0));
    }
    let Some(main) = face.main_script() else {
        return lines;
    };
    let sample = main.sample.unwrap_or_default();
    for size in [48.0, 32.0, 18.0] {
        lines.push(Line::new(sample, size));
    }
    if main == LATIN {
        lines.push(Line::new(ALPHABET, 22.0));
        if face.monospace {
            for code in CODE {
                lines.push(Line::new(code, 18.0));
            }
        }
        if let Some(wght) = face.axes.iter().find(|a| a.tag == "wght")
            && wght.min < wght.max
        {
            for value in [wght.min, wght.max] {
                lines.push(Line {
                    variation: Some(variation(face, "wght", value)),
                    ..Line::new(format!("Handgloves {value:.0}"), 32.0)
                });
            }
        }
    }
    for other in face.other_scripts().into_iter().take(2) {
        lines.push(Line {
            soft: true,
            ..Line::new(other.sample.unwrap_or_default(), 24.0)
        });
    }
    lines
}

/// `font-variation-settings` with every axis at its default, except `tag`.
fn variation(face: &Face, tag: &str, value: f32) -> String {
    face.axes
        .iter()
        .filter(|a| a.tag.len() == 4 && a.tag.chars().all(|c| c.is_ascii_alphanumeric()))
        .map(|a| {
            let v = if a.tag == tag { value } else { a.default };
            format!("'{}' {v}", a.tag)
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn xml_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}

fn svg(lines: &[Line], default_variation: &str) -> (String, f32) {
    let mut body = String::new();
    let mut y = PAD;
    for line in lines {
        y += line.size * 1.05;
        let variation = line.variation.as_deref().unwrap_or(default_variation);
        body.push_str(&format!(
            "<text x=\"{PAD}\" y=\"{y:.1}\" font-size=\"{}\" fill=\"{}\"{}>{}</text>\n",
            line.size,
            if line.soft { SOFT_INK } else { INK },
            if variation.is_empty() {
                String::new()
            } else {
                format!(" font-variation-settings=\"{variation}\"")
            },
            xml_escape(&line.text),
        ));
        y += line.size * 0.35;
    }
    let height = y + PAD;
    let doc = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{MAX_WIDTH}\" height=\"{height:.0}\">\n\
         <g font-family=\"specimen\" font-style=\"normal\" xml:space=\"preserve\">\n{body}</g>\n</svg>\n"
    );
    (doc, height)
}

/// A font database with only face `index` of `sfnt`, and options that always pick it.
fn options(sfnt: Arc<Vec<u8>>, index: u32) -> Result<usvg::Options<'static>, String> {
    let mut db = usvg::fontdb::Database::new();
    let ids = db.load_font_source(usvg::fontdb::Source::Binary(sfnt));
    let mut keep = None;
    for id in ids {
        match db.face(id).map(|f| f.index) {
            Some(i) if i == index && keep.is_none() => keep = Some(id),
            _ => db.remove_face(id),
        }
    }
    let keep = keep.ok_or("the font cannot be loaded for drawing")?;
    Ok(usvg::Options {
        fontdb: Arc::new(db),
        font_resolver: usvg::FontResolver {
            select_font: Box::new(move |_, _| Some(keep)),
            select_fallback: Box::new(|_, _, _| None),
        },
        ..Default::default()
    })
}

fn draw(svg: &str, opts: &usvg::Options, height: f32, scale: f32) -> Result<RgbaImage, String> {
    let tree = usvg::Tree::from_str(svg, opts).map_err(|e| e.to_string())?;
    let bbox = tree.root().abs_bounding_box();
    if bbox.width() <= 0.0 || bbox.height() <= 0.0 || !tree.root().has_children() {
        return Err("no glyphs to draw".into());
    }
    let width = (bbox.right() + PAD).clamp(1.0, MAX_WIDTH);
    let (w, h) = (
        (width * scale).ceil() as u32,
        (height * scale).ceil() as u32,
    );
    let mut pixmap =
        resvg::tiny_skia::Pixmap::new(w.max(1), h.max(1)).ok_or("specimen too large")?;
    pixmap.fill(resvg::tiny_skia::Color::from_rgba8(
        PAPER.0, PAPER.1, PAPER.2, 255,
    ));
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    if blank(&pixmap) {
        return Err("its glyphs cannot be drawn (bitmap-only fonts are not supported)".into());
    }
    RgbaImage::from_raw(w.max(1), h.max(1), pixmap.take_demultiplied())
        .ok_or_else(|| "specimen render failed".to_string())
}

/// True when nothing but paper was painted (glyphs without outlines, such as CBDT
/// colour bitmaps, which resvg does not draw).
fn blank(pixmap: &resvg::tiny_skia::Pixmap) -> bool {
    pixmap
        .pixels()
        .iter()
        .all(|p| (p.red(), p.green(), p.blue()) == PAPER)
}

/// Renders the specimen of face `index`. `scale` multiplies the nominal pixel sizes
/// (48/32/18 px), for screens with large cells.
pub fn render(
    sfnt: Arc<Vec<u8>>,
    index: u32,
    face: &Face,
    scale: f32,
) -> Result<RgbaImage, String> {
    let font = FontRef::from_index(&sfnt, index).map_err(|e| e.to_string())?;
    let lines = lines(&font, face);
    if lines.is_empty() {
        return Err("no sample text for this font".into());
    }
    let default_variation = match face.axes.first() {
        Some(first) => variation(face, &first.tag, first.default),
        None => String::new(),
    };
    let (doc, height) = svg(&lines, &default_variation);
    let opts = options(sfnt.clone(), index)?;
    draw(&doc, &opts, height, scale.clamp(0.25, 4.0))
}

/// A square tile with a big `Aa` (or a character of the font's main script), for
/// directory thumbnails.
pub fn thumbnail(sfnt: Arc<Vec<u8>>, index: u32, face: &Face, side: u32) -> Option<RgbaImage> {
    let font = FontRef::from_index(&sfnt, index).ok()?;
    let text = match face.main_script() {
        Some(s) if s == LATIN => "Aa".to_string(),
        Some(s) => s
            .sample
            .unwrap_or_default()
            .chars()
            .filter(|c| !c.is_whitespace())
            .take(2)
            .collect(),
        None => return None,
    };
    if !covers(&font, &text) {
        return None;
    }
    let default_variation = match face.axes.first() {
        Some(first) => variation(face, &first.tag, first.default),
        None => String::new(),
    };
    let doc = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"100\" height=\"100\">\
         <text x=\"50\" y=\"68\" font-family=\"specimen\" font-size=\"52\" text-anchor=\"middle\" fill=\"{INK}\"{}>{}</text></svg>",
        if default_variation.is_empty() {
            String::new()
        } else {
            format!(" font-variation-settings=\"{default_variation}\"")
        },
        xml_escape(&text)
    );
    let opts = options(sfnt.clone(), index).ok()?;
    let tree = usvg::Tree::from_str(&doc, &opts).ok()?;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(side, side)?;
    pixmap.fill(resvg::tiny_skia::Color::from_rgba8(
        PAPER.0, PAPER.1, PAPER.2, 255,
    ));
    let scale = side as f32 / 100.0;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    if blank(&pixmap) {
        return None;
    }
    RgbaImage::from_raw(side, side, pixmap.take_demultiplied())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_is_escaped_for_xml() {
        assert_eq!(
            xml_escape("a<b>&\"'\x1b[31m"),
            "a&lt;b&gt;&amp;&quot;&apos;[31m"
        );
    }

    #[test]
    fn variation_settings_keep_other_axes_at_default() {
        let face = Face {
            axes: vec![
                super::super::info::Axis {
                    tag: "wght".into(),
                    min: 100.0,
                    default: 400.0,
                    max: 900.0,
                },
                super::super::info::Axis {
                    tag: "wdth".into(),
                    min: 75.0,
                    default: 100.0,
                    max: 100.0,
                },
                super::super::info::Axis {
                    tag: "b'd".into(),
                    min: 0.0,
                    default: 0.0,
                    max: 1.0,
                },
            ],
            ..Face::default()
        };
        assert_eq!(variation(&face, "wght", 900.0), "'wght' 900, 'wdth' 100");
    }
}
