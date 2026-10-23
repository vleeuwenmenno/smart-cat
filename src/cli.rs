use std::path::PathBuf;

use clap::Parser;

use crate::config::{ColorWhen, ListStyle, PagingMode, Protocol};

/// A smarter cat: prints files like cat, renders them when the output is a terminal.
///
/// When standard output is not a terminal, scat writes exactly what cat would.
#[derive(Parser, Debug)]
#[command(
    name = "scat",
    version,
    max_term_width = 100,
    args_override_self = true
)]
pub struct Cli {
    /// Files to print. With no FILE, or when FILE is -, read standard input.
    #[arg(value_name = "FILE")]
    pub files: Vec<PathBuf>,

    /// Equivalent to -vET
    #[arg(short = 'A', long = "show-all", help_heading = "cat options")]
    pub show_all: bool,

    /// Number nonempty output lines, overrides -n
    #[arg(short = 'b', long = "number-nonblank", help_heading = "cat options")]
    pub number_nonblank: bool,

    /// Equivalent to -vE
    #[arg(short = 'e', help_heading = "cat options")]
    pub show_ends_nonprinting: bool,

    /// Display $ at end of each line
    #[arg(short = 'E', long = "show-ends", help_heading = "cat options")]
    pub show_ends: bool,

    /// Number all output lines
    #[arg(
        short = 'n',
        long = "number",
        visible_alias = "line-numbers",
        help_heading = "cat options"
    )]
    pub number: bool,

    /// Suppress repeated empty output lines
    #[arg(short = 's', long = "squeeze-blank", help_heading = "cat options")]
    pub squeeze_blank: bool,

    /// Equivalent to -vT
    #[arg(short = 't', help_heading = "cat options")]
    pub show_tabs_nonprinting: bool,

    /// Display TAB characters as ^I
    #[arg(short = 'T', long = "show-tabs", help_heading = "cat options")]
    pub show_tabs: bool,

    /// (ignored)
    #[arg(short = 'u', help_heading = "cat options")]
    pub unbuffered: bool,

    /// Use ^ and M- notation, except for LFD and TAB
    #[arg(short = 'v', long = "show-nonprinting", help_heading = "cat options")]
    pub show_nonprinting: bool,

    /// Render even when standard output is not a terminal
    #[arg(short = 'f', long, help_heading = "Rendering")]
    pub force_render: bool,

    /// Never render: behave exactly like cat
    #[arg(long, conflicts_with = "force_render", help_heading = "Rendering")]
    pub raw: bool,

    /// Render without decorations: no line number gutter
    #[arg(short = 'p', long, help_heading = "Rendering")]
    pub plain: bool,

    /// When to page long output ($SCAT_PAGER, then $PAGER, default `less -RFX`)
    #[arg(
        long,
        value_enum,
        value_name = "WHEN",
        env = "SCAT_PAGING",
        help_heading = "Rendering"
    )]
    pub paging: Option<PagingMode>,

    /// Show hidden (dot) entries when listing a directory
    #[arg(long, help_heading = "Directories")]
    pub all: bool,

    /// List directories one entry per line, with size and age (same as --style long)
    #[arg(long, help_heading = "Directories")]
    pub long: bool,

    /// How to list directories
    #[arg(long, value_enum, value_name = "STYLE", help_heading = "Directories")]
    pub style: Option<ListStyle>,

    /// Show icons instead of thumbnails in the tiles view
    #[arg(long, help_heading = "Directories")]
    pub no_thumbnails: bool,

    /// Thumbnail box of a tile in cells, for example 20x10
    #[arg(long, value_name = "WxH", value_parser = parse_tile_size, help_heading = "Directories")]
    pub tile_size: Option<(u16, u16)>,

    /// When to use colors
    #[arg(
        long,
        value_enum,
        value_name = "WHEN",
        env = "SCAT_COLOR",
        help_heading = "Rendering"
    )]
    pub color: Option<ColorWhen>,

    /// Syntax highlighting theme (see --list-themes)
    #[arg(
        long,
        value_name = "NAME",
        env = "SCAT_THEME",
        help_heading = "Rendering"
    )]
    pub theme: Option<String>,

    /// Force the syntax used for highlighting, by name or extension (see --list-languages)
    #[arg(short = 'l', long, value_name = "LANG", help_heading = "Rendering")]
    pub language: Option<String>,

    /// Play audio and video files with mpv instead of showing their info
    #[arg(long, conflicts_with = "raw", help_heading = "Media")]
    pub play: bool,
    /// How to draw images
    #[arg(
        long,
        value_enum,
        value_name = "METHOD",
        env = "SCAT_PROTOCOL",
        help_heading = "Rendering"
    )]
    pub protocol: Option<Protocol>,

    /// Show only the first frame of animated images
    #[arg(long, help_heading = "Rendering")]
    pub no_animate: bool,

    /// PDF pages to show: 1, 1-3,7, 4- or all (default: [pdf] pages, usually 1)
    #[arg(long, value_name = "SPEC", value_parser = crate::pdf::PageSpec::parse, help_heading = "Documents")]
    pub pages: Option<crate::pdf::PageSpec>,

    /// Show the text of PDF pages instead of drawing them
    #[arg(long, help_heading = "Documents")]
    pub text: bool,

    /// Use this config file instead of the default location
    #[arg(
        long,
        value_name = "PATH",
        env = "SCAT_CONFIG",
        help_heading = "Config"
    )]
    pub config: Option<PathBuf>,

    /// Ignore the config file
    #[arg(long, help_heading = "Config")]
    pub no_config: bool,

    /// Write a commented config template to the config path
    #[arg(long, help_heading = "Config")]
    pub init_config: bool,

    /// Print the effective configuration as TOML
    #[arg(long, help_heading = "Config")]
    pub print_config: bool,

    /// Report terminal capabilities, config and, for each FILE, how it would be rendered
    #[arg(long, help_heading = "Info")]
    pub diagnose: bool,

    /// List available syntax highlighting themes
    #[arg(long, help_heading = "Info")]
    pub list_themes: bool,

    /// List languages that can be highlighted
    #[arg(long, help_heading = "Info")]
    pub list_languages: bool,

    /// Print a shell completion script (for example: scat --completions fish)
    #[arg(long, value_name = "SHELL", value_enum, help_heading = "Info")]
    pub completions: Option<clap_complete::Shell>,

    /// Print the man page (roff)
    #[arg(long, help_heading = "Info")]
    pub man: bool,

    /// Install completions for your shells (fish, bash, zsh) and the man page
    #[arg(long, help_heading = "Install")]
    pub install_completions: bool,

    /// Remove the completions and man page installed by --install-completions
    #[arg(long, conflicts_with = "install_completions", help_heading = "Install")]
    pub uninstall_completions: bool,

    /// Rewrite completion and man page files that already exist (used after updates)
    #[arg(long, hide = true)]
    pub refresh_completions: bool,

    /// Update scat to the newest release
    #[arg(long, help_heading = "Updates")]
    pub update: bool,

    /// Check whether a newer release exists, without installing it
    #[arg(long, conflicts_with = "update", help_heading = "Updates")]
    pub check_update: bool,

    /// Include pre-releases when updating or checking
    #[arg(long, help_heading = "Updates")]
    pub pre: bool,
}

impl Cli {
    /// True when any of cat's visualisation flags (-A, -e, -E, -t, -T, -v) is set.
    /// Those ask for cat's exact output, so scat does not render.
    pub fn wants_cat_visualisation(&self) -> bool {
        self.show_all
            || self.show_ends_nonprinting
            || self.show_ends
            || self.show_tabs_nonprinting
            || self.show_tabs
            || self.show_nonprinting
    }
}

/// Parses `WxH` with both sides between 4 and 200 cells.
fn parse_tile_size(text: &str) -> Result<(u16, u16), String> {
    let (w, h) = text
        .split_once(['x', 'X'])
        .ok_or_else(|| "expected WxH, for example 16x8".to_string())?;
    let side = |s: &str| -> Result<u16, String> {
        let n: u16 = s.trim().parse().map_err(|_| format!("not a number: {s}"))?;
        if (4..=200).contains(&n) {
            Ok(n)
        } else {
            Err(format!("{n} is out of range (4 to 200)"))
        }
    };
    Ok((side(w)?, side(h)?))
}

#[cfg(test)]
mod tests {
    use super::parse_tile_size;

    #[test]
    fn tile_size_parses() {
        assert_eq!(parse_tile_size("16x8"), Ok((16, 8)));
        assert_eq!(parse_tile_size("20X10"), Ok((20, 10)));
        assert!(parse_tile_size("16").is_err());
        assert!(parse_tile_size("2x8").is_err());
        assert!(parse_tile_size("ax8").is_err());
    }
}
