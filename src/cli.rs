use std::path::PathBuf;

use clap::Parser;

use crate::config::ColorWhen;

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

    /// Show hidden (dot) entries when listing a directory
    #[arg(long, help_heading = "Directories")]
    pub all: bool,

    /// List directories one entry per line, with size and age
    #[arg(long, help_heading = "Directories")]
    pub long: bool,

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
