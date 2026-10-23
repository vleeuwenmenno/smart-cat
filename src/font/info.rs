//! Facts about one font face: names, classes, axes, coverage and features.
//!
//! Every string comes from the font and is untrusted; callers print them through
//! [`crate::kv`], which sanitizes.

use skrifa::raw::TableProvider;
use skrifa::raw::tables::os2::SelectionFlags;
use skrifa::string::StringId;
use skrifa::{FontRef, MetadataProvider, Tag};

/// A script (or a group of blocks) counted in the coverage summary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Script {
    pub name: &'static str,
    /// A short sample for the specimen, when the script has one.
    pub sample: Option<&'static str>,
    /// Shown in the summary but never chosen as the specimen's main script.
    pub symbols: bool,
}

const fn script(name: &'static str, sample: &'static str) -> Script {
    Script {
        name,
        sample: Some(sample),
        symbols: false,
    }
}

const fn symbols(name: &'static str) -> Script {
    Script {
        name,
        sample: None,
        symbols: true,
    }
}

pub const LATIN: Script = script("Latin", "The quick brown fox jumps over the lazy dog");
const GREEK: Script = script("Greek", "Ξεσκεπάζω την ψυχοφθόρα βδελυγμία");
const CYRILLIC: Script = script("Cyrillic", "Съешь же ещё этих мягких французских булок");
const ARMENIAN: Script = script("Armenian", "Բարեւ աշխարհ");
const HEBREW: Script = script("Hebrew", "דג סקרן שט בים מאוכזב ולפתע מצא חברה");
const ARABIC: Script = script("Arabic", "نص حكيم له سر قاطع وذو شأن عظيم");
const DEVANAGARI: Script = script("Devanagari", "ऋषियों को सताने वाले दुष्ट राक्षसों के राजा रावण");
const BENGALI: Script = script("Bengali", "আমার সোনার বাংলা আমি তোমায় ভালোবাসি");
const TAMIL: Script = script("Tamil", "யாமறிந்த மொழிகளிலே தமிழ்மொழி போல்");
const INDIC: Script = script("Indic (other)", "ਪੰਜਾਬੀ ગુજરાતી ଓଡ଼ିଆ తెలుగు ಕನ್ನಡ മലയാളം");
const THAI: Script = script("Thai", "เป็นมนุษย์สุดประเสริฐเลิศคุณค่า");
const GEORGIAN: Script = script("Georgian", "სწრაფი ყავისფერი მელა");
const ETHIOPIC: Script = script("Ethiopic", "ሰላም ልዑል ዓለም");
const KHMER: Script = script("Khmer", "ខ្ញុំអាចញ៉ាំកញ្ចក់បាន");
const HANGUL: Script = script("Hangul", "다람쥐 헌 쳇바퀴에 타고파");
const KANA: Script = script("Kana", "いろはにほへと ちりぬるを カタカナ");
const CJK: Script = script("CJK", "永和九年，岁在癸丑，暮春之初");
const EMOJI: Script = script("Emoji", "😀 🎉 🚀 🐱 🍕 🌍 ❤");
const PUNCTUATION: Script = symbols("Punctuation & symbols");
const BOX: Script = symbols("Box drawing");
const PRIVATE: Script = symbols("Private use");

/// Unicode ranges per script, roughly by block.
const RANGES: &[(u32, u32, Script)] = &[
    (0x0041, 0x005a, LATIN),
    (0x0061, 0x007a, LATIN),
    (0x00c0, 0x024f, LATIN),
    (0x1e00, 0x1eff, LATIN),
    (0x2c60, 0x2c7f, LATIN),
    (0xa720, 0xa7ff, LATIN),
    (0xab30, 0xab6f, LATIN),
    (0x0370, 0x03ff, GREEK),
    (0x1f00, 0x1fff, GREEK),
    (0x0400, 0x052f, CYRILLIC),
    (0x1c80, 0x1c8f, CYRILLIC),
    (0x2de0, 0x2dff, CYRILLIC),
    (0xa640, 0xa69f, CYRILLIC),
    (0x0530, 0x058f, ARMENIAN),
    (0x0590, 0x05ff, HEBREW),
    (0xfb1d, 0xfb4f, HEBREW),
    (0x0600, 0x06ff, ARABIC),
    (0x0750, 0x077f, ARABIC),
    (0x08a0, 0x08ff, ARABIC),
    (0xfb50, 0xfdff, ARABIC),
    (0xfe70, 0xfeff, ARABIC),
    (0x0900, 0x097f, DEVANAGARI),
    (0xa8e0, 0xa8ff, DEVANAGARI),
    (0x0980, 0x09ff, BENGALI),
    (0x0b80, 0x0bff, TAMIL),
    (0x0a00, 0x0b7f, INDIC),
    (0x0c00, 0x0dff, INDIC),
    (0x0e00, 0x0e7f, THAI),
    (0x10a0, 0x10ff, GEORGIAN),
    (0x1200, 0x139f, ETHIOPIC),
    (0x1780, 0x17ff, KHMER),
    (0x1100, 0x11ff, HANGUL),
    (0x3130, 0x318f, HANGUL),
    (0xac00, 0xd7af, HANGUL),
    (0x3040, 0x30ff, KANA),
    (0x31f0, 0x31ff, KANA),
    (0x2e80, 0x2fdf, CJK),
    (0x3000, 0x303f, CJK),
    (0x3400, 0x4dbf, CJK),
    (0x4e00, 0x9fff, CJK),
    (0xf900, 0xfaff, CJK),
    (0x20000, 0x3ffff, CJK),
    (0x2500, 0x259f, BOX),
    (0x2800, 0x28ff, BOX),
    (0x1fb00, 0x1fbff, BOX),
    (0x1f300, 0x1f5ff, EMOJI),
    (0x1f600, 0x1f64f, EMOJI),
    (0x1f680, 0x1f6ff, EMOJI),
    (0x1f900, 0x1f9ff, EMOJI),
    (0x1fa70, 0x1faff, EMOJI),
    (0x0021, 0x0040, PUNCTUATION),
    (0x005b, 0x0060, PUNCTUATION),
    (0x007b, 0x007e, PUNCTUATION),
    (0x00a1, 0x00bf, PUNCTUATION),
    (0x2000, 0x24ff, PUNCTUATION),
    (0x25a0, 0x27bf, PUNCTUATION),
    (0x2900, 0x2bff, PUNCTUATION),
    (0xe000, 0xf8ff, PRIVATE),
    (0xf0000, 0x10ffff, PRIVATE),
];

fn script_of(cp: u32) -> Option<Script> {
    RANGES
        .iter()
        .find(|(lo, hi, _)| (*lo..=*hi).contains(&cp))
        .map(|r| r.2)
}

/// A variation axis: tag, minimum, default, maximum.
#[derive(Debug, Clone, PartialEq)]
pub struct Axis {
    pub tag: String,
    pub min: f32,
    pub default: f32,
    pub max: f32,
}

#[derive(Debug, Clone, Default)]
pub struct Face {
    pub family: String,
    pub subfamily: String,
    pub full_name: String,
    pub postscript_name: String,
    pub version: String,
    pub designer: String,
    pub manufacturer: String,
    pub license: String,
    pub license_url: String,
    pub weight: u16,
    pub width: u16,
    pub italic: bool,
    pub monospace: bool,
    pub axes: Vec<Axis>,
    pub instances: usize,
    pub glyphs: u32,
    pub units_per_em: u16,
    pub outlines: &'static str,
    /// Mapped characters.
    pub characters: usize,
    /// Characters per script, most first.
    pub coverage: Vec<(Script, usize)>,
    /// How many of `A-Z a-z` are mapped.
    pub basic_latin: usize,
    /// GSUB and GPOS feature tags, sorted and unique.
    pub features: Vec<String>,
    /// Colour and bitmap glyph tables present.
    pub color: Vec<&'static str>,
}

impl Face {
    /// The script the specimen shows: Latin when nearly all of `A-Z a-z` is there,
    /// else the best covered script that has a sample.
    pub fn main_script(&self) -> Option<Script> {
        if self.basic_latin >= 40 {
            return Some(LATIN);
        }
        self.coverage
            .iter()
            .find(|(s, _)| !s.symbols && s.sample.is_some() && *s != LATIN)
            .map(|(s, _)| *s)
            .or_else(|| (self.basic_latin > 0).then_some(LATIN))
    }

    /// Other scripts with a real share of the font, for extra specimen lines.
    pub fn other_scripts(&self) -> Vec<Script> {
        let main = self.main_script();
        self.coverage
            .iter()
            .filter(|(s, n)| Some(*s) != main && !s.symbols && s.sample.is_some() && *n >= 40)
            .map(|(s, _)| *s)
            .collect()
    }
}

fn name(font: &FontRef, ids: &[StringId]) -> String {
    for &id in ids {
        if let Some(s) = font.localized_strings(id).english_or_first() {
            let text: String = s.chars().collect();
            let text = text.trim();
            if !text.is_empty() {
                return text.to_string();
            }
        }
    }
    String::new()
}

/// The first line of a long name entry, cut to `max` characters.
fn first_line(text: &str, max: usize) -> String {
    let line = text
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("");
    if line.chars().count() > max {
        let mut cut: String = line.chars().take(max).collect();
        cut.push('…');
        cut
    } else {
        line.to_string()
    }
}

/// A printable tag (feature or axis); other bytes become `?`.
fn tag_text(tag: Tag) -> String {
    tag.to_be_bytes()
        .iter()
        .map(|&b| {
            if b.is_ascii_graphic() || b == b' ' {
                char::from(b)
            } else {
                '?'
            }
        })
        .collect::<String>()
        .trim_end()
        .to_string()
}

pub fn describe(font: &FontRef) -> Face {
    let mut face = Face {
        family: name(
            font,
            &[StringId::TYPOGRAPHIC_FAMILY_NAME, StringId::FAMILY_NAME],
        ),
        subfamily: name(
            font,
            &[
                StringId::TYPOGRAPHIC_SUBFAMILY_NAME,
                StringId::SUBFAMILY_NAME,
            ],
        ),
        full_name: name(font, &[StringId::FULL_NAME]),
        postscript_name: name(font, &[StringId::POSTSCRIPT_NAME]),
        version: name(font, &[StringId::VERSION_STRING]),
        designer: first_line(&name(font, &[StringId::DESIGNER]), 100),
        manufacturer: first_line(&name(font, &[StringId::MANUFACTURER]), 100),
        license: first_line(&name(font, &[StringId::LICENSE_DESCRIPTION]), 120),
        license_url: first_line(&name(font, &[StringId::LICENSE_URL]), 120),
        ..Face::default()
    };
    if let Ok(os2) = font.os2() {
        face.weight = os2.us_weight_class();
        face.width = os2.us_width_class();
        face.italic = os2
            .fs_selection()
            .intersects(SelectionFlags::ITALIC | SelectionFlags::OBLIQUE);
    } else {
        let attrs = font.attributes();
        face.weight = attrs.weight.value().round().clamp(1.0, 1000.0) as u16;
        face.italic = attrs.style != skrifa::attribute::Style::Normal;
    }
    let metrics = font.metrics(
        skrifa::instance::Size::unscaled(),
        skrifa::instance::LocationRef::default(),
    );
    face.units_per_em = metrics.units_per_em;
    face.monospace = metrics.is_monospace || font.post().is_ok_and(|p| p.is_fixed_pitch() != 0);
    face.italic |= metrics.italic_angle != 0.0;
    face.glyphs = font.maxp().map_or(u32::from(metrics.glyph_count), |m| {
        u32::from(m.num_glyphs())
    });
    face.axes = font
        .axes()
        .iter()
        .map(|a| Axis {
            tag: tag_text(a.tag()),
            min: a.min_value(),
            default: a.default_value(),
            max: a.max_value(),
        })
        .collect();
    face.instances = font.named_instances().len();
    let has = |t: &[u8; 4]| font.table_data(Tag::new(t)).is_some();
    face.outlines = if has(b"CFF2") {
        "CFF2"
    } else if has(b"CFF ") {
        "CFF (PostScript)"
    } else if has(b"glyf") {
        "TrueType"
    } else {
        "none (bitmap or colour only)"
    };
    for (tag, label) in [
        (b"COLR", "COLR"),
        (b"SVG ", "SVG"),
        (b"CBDT", "CBDT bitmaps"),
        (b"sbix", "sbix bitmaps"),
        (b"EBDT", "EBDT bitmaps"),
    ] {
        if has(tag) {
            face.color.push(label);
        }
    }

    let mut counts: Vec<(Script, usize)> = Vec::new();
    for (cp, _) in font.charmap().mappings() {
        face.characters += 1;
        if (0x41..=0x5a).contains(&cp) || (0x61..=0x7a).contains(&cp) {
            face.basic_latin += 1;
        }
        if let Some(s) = script_of(cp) {
            match counts.iter_mut().find(|(c, _)| c.name == s.name) {
                Some(entry) => entry.1 += 1,
                None => counts.push((s, 1)),
            }
        }
    }
    // A stray character or two (a currency sign, a mark) is not coverage.
    counts.retain(|(_, n)| *n >= 3);
    counts.sort_by_key(|&(_, n)| std::cmp::Reverse(n));
    face.coverage = counts;

    let mut features: Vec<String> = Vec::new();
    let mut add = |list: Result<skrifa::raw::tables::layout::FeatureList, _>| {
        if let Ok(list) = list {
            for rec in list.feature_records() {
                features.push(tag_text(rec.feature_tag()));
            }
        }
    };
    if let Ok(gsub) = font.gsub() {
        add(gsub.feature_list());
    }
    if let Ok(gpos) = font.gpos() {
        add(gpos.feature_list());
    }
    features.sort();
    features.dedup();
    face.features = features;
    face
}

fn weight_name(w: u16) -> &'static str {
    match w {
        0..=149 => "Thin",
        150..=249 => "ExtraLight",
        250..=349 => "Light",
        350..=449 => "Regular",
        450..=549 => "Medium",
        550..=649 => "SemiBold",
        650..=749 => "Bold",
        750..=849 => "ExtraBold",
        _ => "Black",
    }
}

fn width_name(w: u16) -> &'static str {
    match w {
        1 => "UltraCondensed",
        2 => "ExtraCondensed",
        3 => "Condensed",
        4 => "SemiCondensed",
        5 => "Normal",
        6 => "SemiExpanded",
        7 => "Expanded",
        8 => "ExtraExpanded",
        9 => "UltraExpanded",
        _ => "unknown",
    }
}

fn number(v: f32) -> String {
    if v.fract() == 0.0 {
        format!("{v:.0}")
    } else {
        format!("{v:.1}")
    }
}

/// Most features listed before `… N more`.
const MAX_FEATURES: usize = 24;
/// Most scripts listed in the coverage line.
const MAX_SCRIPTS: usize = 8;

/// The table rows for one face. `dash` is `–` or `-` for ranges.
pub fn rows(face: &Face, utf8: bool) -> Vec<(&'static str, String)> {
    let dash = if utf8 { "–" } else { "-" };
    let sep = if utf8 { " · " } else { ", " };
    let yes_no = |b: bool| if b { "yes" } else { "no" }.to_string();
    let mut rows = vec![
        ("family", face.family.clone()),
        ("style", face.subfamily.clone()),
        ("full name", face.full_name.clone()),
        ("PostScript", face.postscript_name.clone()),
        ("version", face.version.clone()),
        ("designer", face.designer.clone()),
        ("manufacturer", face.manufacturer.clone()),
        (
            "weight",
            if face.weight > 0 {
                format!("{} ({})", face.weight, weight_name(face.weight))
            } else {
                String::new()
            },
        ),
        (
            "width",
            if face.width > 0 {
                format!("{} ({})", face.width, width_name(face.width))
            } else {
                String::new()
            },
        ),
        ("italic", yes_no(face.italic)),
        ("monospace", yes_no(face.monospace)),
    ];
    for (i, a) in face.axes.iter().enumerate() {
        rows.push((
            if i == 0 { "axes" } else { "" },
            format!(
                "{} {}{dash}{}, default {}",
                a.tag,
                number(a.min),
                number(a.max),
                number(a.default)
            ),
        ));
    }
    if face.instances > 0 {
        rows.push(("instances", face.instances.to_string()));
    }
    rows.push(("outlines", face.outlines.to_string()));
    if !face.color.is_empty() {
        rows.push(("colour", face.color.join(", ")));
    }
    rows.push(("glyphs", crate::table::thousands(u64::from(face.glyphs))));
    rows.push(("units/em", face.units_per_em.to_string()));
    rows.push((
        "characters",
        crate::table::thousands(face.characters as u64),
    ));
    let mut coverage: Vec<String> = face
        .coverage
        .iter()
        .take(MAX_SCRIPTS)
        .map(|(s, n)| format!("{} {}", s.name, crate::table::thousands(*n as u64)))
        .collect();
    if face.coverage.len() > MAX_SCRIPTS {
        coverage.push(format!("+{} more", face.coverage.len() - MAX_SCRIPTS));
    }
    rows.push(("coverage", coverage.join(sep)));
    let mut features: Vec<String> = face.features.iter().take(MAX_FEATURES).cloned().collect();
    if face.features.len() > MAX_FEATURES {
        features.push(format!("+{} more", face.features.len() - MAX_FEATURES));
    }
    rows.push(("features", features.join(" ")));
    rows.push(("license", face.license.clone()));
    rows.push(("license URL", face.license_url.clone()));
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scripts_by_block() {
        assert_eq!(script_of('a' as u32), Some(LATIN));
        assert_eq!(script_of('Ж' as u32).map(|s| s.name), Some("Cyrillic"));
        assert_eq!(script_of('漢' as u32).map(|s| s.name), Some("CJK"));
        assert_eq!(script_of(0xe0a0).map(|s| s.name), Some("Private use"));
        assert_eq!(script_of('\u{0}' as u32), None);
    }

    #[test]
    fn long_names_keep_their_first_line() {
        assert_eq!(first_line("\n  OFL 1.1\nmore text", 50), "OFL 1.1");
        assert_eq!(first_line("abcdef", 3), "abc…");
    }

    #[test]
    fn main_script_prefers_latin() {
        let mut face = Face {
            basic_latin: 52,
            coverage: vec![(CYRILLIC, 200), (LATIN, 52)],
            ..Face::default()
        };
        assert_eq!(face.main_script(), Some(LATIN));
        assert_eq!(face.other_scripts(), vec![CYRILLIC]);
        face.basic_latin = 0;
        face.coverage = vec![(PUNCTUATION, 300), (ARABIC, 250)];
        assert_eq!(face.main_script(), Some(ARABIC));
    }
}
