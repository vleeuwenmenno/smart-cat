//! Syntax highlighting with syntect and bat's syntax and theme sets (via two-face).

use std::io::{self, Write};

use syntect::highlighting::{Color, FontStyle, Style, Theme};
use syntect::parsing::{SyntaxReference, SyntaxSet};
use two_face::theme::LazyThemeSet;

use crate::term::ColorDepth;

pub struct Assets {
    pub syntaxes: SyntaxSet,
    themes: LazyThemeSet,
}

impl Assets {
    pub fn load() -> Self {
        Self {
            syntaxes: two_face::syntax::extra_newlines(),
            themes: LazyThemeSet::from(two_face::theme::extra()),
        }
    }

    pub fn theme(&self, name: &str) -> Option<&Theme> {
        self.themes.get(name).or_else(|| {
            let wanted = name.to_ascii_lowercase();
            let actual = self
                .themes
                .theme_names()
                .find(|n| n.to_ascii_lowercase() == wanted)?;
            self.themes.get(actual)
        })
    }

    pub fn theme_names(&self) -> Vec<&str> {
        let mut names: Vec<&str> = self.themes.theme_names().collect();
        names.sort_unstable_by_key(|n| n.to_ascii_lowercase());
        names
    }

    /// Picks a syntax from an explicit language, then the file name, then the first line
    /// (shebangs, modelines, `<?xml`). Plain text counts as no syntax.
    pub fn find_syntax(
        &self,
        language: Option<&str>,
        file_name: Option<&str>,
        first_line: &str,
    ) -> Option<&SyntaxReference> {
        let found = if let Some(language) = language {
            self.syntaxes.find_syntax_by_token(language)
        } else {
            file_name
                .and_then(|name| self.by_file_name(name))
                .or_else(|| self.syntaxes.find_syntax_by_first_line(first_line))
        };
        found.filter(|s| s.name != "Plain Text")
    }

    fn by_file_name(&self, name: &str) -> Option<&SyntaxReference> {
        let s = &self.syntaxes;
        s.find_syntax_by_extension(name)
            .or_else(|| s.find_syntax_by_extension(name.trim_start_matches('.')))
            .or_else(|| {
                let (_, ext) = name.rsplit_once('.')?;
                s.find_syntax_by_extension(ext)
                    .or_else(|| s.find_syntax_by_extension(&ext.to_ascii_lowercase()))
            })
    }
}

pub fn list_themes() -> i32 {
    let assets = Assets::load();
    let mut out = io::stdout().lock();
    for name in assets.theme_names() {
        if writeln!(out, "{name}").is_err() {
            return 1;
        }
    }
    0
}

pub fn list_languages() -> i32 {
    let assets = Assets::load();
    let mut syntaxes: Vec<_> = assets
        .syntaxes
        .syntaxes()
        .iter()
        .filter(|s| !s.hidden && !s.file_extensions.is_empty())
        .collect();
    syntaxes.sort_unstable_by_key(|s| s.name.to_ascii_lowercase());
    let mut out = io::stdout().lock();
    for syntax in syntaxes {
        if writeln!(
            out,
            "{}: {}",
            syntax.name,
            syntax.file_extensions.join(", ")
        )
        .is_err()
        {
            return 1;
        }
    }
    0
}

/// One SGR sequence that resets the previous style and applies `style`.
pub fn style_sgr(style: Style, depth: ColorDepth) -> String {
    let mut params = vec!["0".to_string()];
    if style.font_style.contains(FontStyle::BOLD) {
        params.push("1".into());
    }
    if style.font_style.contains(FontStyle::ITALIC) {
        params.push("3".into());
    }
    if style.font_style.contains(FontStyle::UNDERLINE) {
        params.push("4".into());
    }
    if let Some(fg) = foreground(style.foreground, depth) {
        params.push(fg);
    }
    format!("\x1b[{}m", params.join(";"))
}

/// bat's "ansi" themes store a palette index in `r` with alpha 0, and alpha 1 means
/// "terminal default". Everything else is a real RGB colour.
fn foreground(color: Color, depth: ColorDepth) -> Option<String> {
    match color.a {
        0 => Some(match color.r {
            0..=7 => format!("{}", 30 + color.r),
            8..=15 => format!("{}", 90 + color.r - 8),
            n => format!("38;5;{n}"),
        }),
        1 => None,
        _ => Some(match depth {
            ColorDepth::TrueColor => format!("38;2;{};{};{}", color.r, color.g, color.b),
            ColorDepth::Ansi256 => format!("38;5;{}", rgb_to_256(color.r, color.g, color.b)),
            ColorDepth::Ansi16 => {
                let idx = rgb_to_16(color.r, color.g, color.b);
                if idx < 8 {
                    format!("{}", 30 + idx)
                } else {
                    format!("{}", 90 + idx - 8)
                }
            }
        }),
    }
}

fn rgb_to_256(r: u8, g: u8, b: u8) -> u8 {
    // Greys map better onto the 24-step grey ramp than onto the colour cube.
    if r == g && g == b {
        return match r {
            0..=7 => 16,
            249..=255 => 231,
            v => 232 + ((u16::from(v) - 8) * 24 / 241) as u8,
        };
    }
    let level = |v: u8| -> u8 {
        if v < 48 {
            0
        } else if v < 115 {
            1
        } else {
            ((u16::from(v) - 35) / 40) as u8
        }
    };
    16 + 36 * level(r) + 6 * level(g) + level(b)
}

const ANSI16: [(u8, u8, u8); 16] = [
    (0, 0, 0),
    (205, 0, 0),
    (0, 205, 0),
    (205, 205, 0),
    (0, 0, 238),
    (205, 0, 205),
    (0, 205, 205),
    (229, 229, 229),
    (127, 127, 127),
    (255, 0, 0),
    (0, 255, 0),
    (255, 255, 0),
    (92, 92, 255),
    (255, 0, 255),
    (0, 255, 255),
    (255, 255, 255),
];

fn rgb_to_16(r: u8, g: u8, b: u8) -> u8 {
    let dist = |&(pr, pg, pb): &(u8, u8, u8)| {
        let d = |a: u8, b: u8| (i32::from(a) - i32::from(b)).pow(2);
        d(r, pr) + d(g, pg) + d(b, pb)
    };
    ANSI16
        .iter()
        .enumerate()
        .min_by_key(|(_, c)| dist(c))
        .map(|(i, _)| i as u8)
        .unwrap_or(7)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_syntaxes() {
        let assets = Assets::load();
        let name = |lang: Option<&str>, file: Option<&str>, first: &str| {
            assets
                .find_syntax(lang, file, first)
                .map(|s| s.name.clone())
        };
        assert_eq!(name(None, Some("main.rs"), ""), Some("Rust".into()));
        assert_eq!(name(None, Some("Makefile"), ""), Some("Makefile".into()));
        assert_eq!(
            name(None, Some("script"), "#!/usr/bin/env python3\n"),
            Some("Python".into())
        );
        assert_eq!(name(Some("toml"), Some("x.txt"), ""), Some("TOML".into()));
        assert_eq!(name(None, Some("notes.txt"), "hello\n"), None);
    }

    #[test]
    fn default_themes_exist() {
        let assets = Assets::load();
        let defaults = crate::config::Defaults::default();
        assert!(assets.theme(&defaults.theme_dark).is_some());
        assert!(assets.theme(&defaults.theme_light).is_some());
        assert!(assets.theme("ansi").is_some());
        assert!(assets.theme("nord").is_some());
    }

    #[test]
    fn colour_reduction() {
        assert_eq!(rgb_to_256(255, 0, 0), 196);
        assert_eq!(rgb_to_256(0, 0, 0), 16);
        assert_eq!(rgb_to_16(250, 10, 10), 9);
    }
}
