//! User configuration: `$XDG_CONFIG_HOME/scat/config.toml`.

use std::fmt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Deserializer, Serialize};

const TEMPLATE: &str = include_str!("config_template.toml");

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum ColorWhen {
    #[default]
    Auto,
    Always,
    Never,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Renderer {
    /// Pick a renderer from the detected file type.
    #[default]
    Auto,
    /// Text with syntax highlighting.
    Code,
    /// Text without syntax highlighting.
    Text,
    /// Exactly what cat prints.
    Raw,
    /// Info view for video files (needs ffprobe).
    Video,
}

impl fmt::Display for Renderer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Renderer::Auto => "auto",
            Renderer::Code => "code",
            Renderer::Text => "text",
            Renderer::Raw => "raw",
            Renderer::Video => "video",
        })
    }
}

/// What `scat` does with no file arguments when a person is at the terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum NoArgs {
    /// List the current directory.
    #[default]
    List,
    /// Read standard input, like cat.
    Stdin,
}

/// What to do with a directory argument when rendering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Directories {
    /// List the directory.
    #[default]
    List,
    /// Fail with "Is a directory", like cat.
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ListStyle {
    #[default]
    Grid,
    Long,
}

/// What to do with escape sequences already present in a text file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum AnsiMode {
    /// Keep colour/style sequences (SGR), show every other control character as ^X.
    #[default]
    KeepSgr,
    /// Show every control character, including SGR sequences, as ^X.
    Escape,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Channel {
    #[default]
    Stable,
    Prerelease,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub defaults: Defaults,
    #[serde(rename = "rule", skip_serializing_if = "Vec::is_empty")]
    pub rules: Vec<Rule>,
    pub update: UpdateConfig,
    pub list: ListConfig,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct ListConfig {
    pub style: ListStyle,
    pub show_hidden: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Defaults {
    pub color: ColorWhen,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
    pub theme_dark: String,
    pub theme_light: String,
    pub line_numbers: bool,
    pub tab_width: usize,
    pub ansi: AnsiMode,
    pub max_highlight_bytes: u64,
    pub no_args: NoArgs,
    pub directories: Directories,
}

impl Default for Defaults {
    fn default() -> Self {
        Self {
            color: ColorWhen::Auto,
            theme: None,
            theme_dark: "Monokai Extended".into(),
            theme_light: "Monokai Extended Light".into(),
            line_numbers: true,
            tab_width: 4,
            ansi: AnsiMode::KeepSgr,
            max_highlight_bytes: 16 * 1024 * 1024,
            no_args: NoArgs::List,
            directories: Directories::List,
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct UpdateConfig {
    pub channel: Channel,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    #[serde(rename = "match", default)]
    pub matcher: Matcher,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub renderer: Option<Renderer>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line_numbers: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tab_width: Option<usize>,
}

/// Which files a rule applies to. Every listed criterion must match; an empty matcher
/// matches every file. Each criterion accepts a string or a list of strings.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Matcher {
    #[serde(
        deserialize_with = "one_or_many",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub ext: Vec<String>,
    #[serde(
        deserialize_with = "one_or_many",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub name: Vec<String>,
    #[serde(
        deserialize_with = "one_or_many",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub mime: Vec<String>,
}

fn one_or_many<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum OneOrMany {
        One(String),
        Many(Vec<String>),
    }
    Ok(match OneOrMany::deserialize(d)? {
        OneOrMany::One(s) => vec![s],
        OneOrMany::Many(v) => v,
    })
}

impl Matcher {
    pub fn matches(&self, file_name: Option<&str>, mime: &str) -> bool {
        let lower = file_name.map(str::to_ascii_lowercase);
        let ext_ok = self.ext.is_empty()
            || lower.as_deref().is_some_and(|name| {
                self.ext.iter().any(|ext| {
                    let ext = ext.trim_start_matches('.').to_ascii_lowercase();
                    name.len() > ext.len() + 1
                        && name.ends_with(&ext)
                        && name.as_bytes()[name.len() - ext.len() - 1] == b'.'
                })
            });
        let name_ok =
            self.name.is_empty() || file_name.is_some_and(|n| self.name.iter().any(|m| m == n));
        let mime_ok = self.mime.is_empty() || self.mime.iter().any(|m| mime_matches(m, mime));
        ext_ok && name_ok && mime_ok
    }
}

/// `image/*` style matching; `*` alone matches everything.
fn mime_matches(pattern: &str, mime: &str) -> bool {
    match pattern.strip_suffix("/*") {
        Some(prefix) => mime.split('/').next() == Some(prefix),
        None => pattern == "*" || pattern.eq_ignore_ascii_case(mime),
    }
}

/// Settings for one file after applying defaults and every matching rule.
#[derive(Debug, Clone)]
pub struct FileSettings {
    pub renderer: Renderer,
    pub language: Option<String>,
    pub line_numbers: bool,
    pub theme: Option<String>,
    pub tab_width: usize,
    /// 1-based indices of the rules that matched, in config order.
    pub matched_rules: Vec<usize>,
}

impl Config {
    /// Applies every matching rule in file order; later rules override earlier ones.
    pub fn settings_for(&self, file_name: Option<&str>, mime: &str) -> FileSettings {
        let mut settings = FileSettings {
            renderer: Renderer::Auto,
            language: None,
            line_numbers: self.defaults.line_numbers,
            theme: self.defaults.theme.clone(),
            tab_width: self.defaults.tab_width,
            matched_rules: Vec::new(),
        };
        for (idx, rule) in self.rules.iter().enumerate() {
            if !rule.matcher.matches(file_name, mime) {
                continue;
            }
            settings.matched_rules.push(idx + 1);
            if let Some(renderer) = rule.renderer {
                settings.renderer = renderer;
            }
            if let Some(language) = &rule.language {
                settings.language = Some(language.clone());
            }
            if let Some(line_numbers) = rule.line_numbers {
                settings.line_numbers = line_numbers;
            }
            if let Some(theme) = &rule.theme {
                settings.theme = Some(theme.clone());
            }
            if let Some(tab_width) = rule.tab_width {
                settings.tab_width = tab_width;
            }
        }
        settings
    }
}

pub struct Loaded {
    pub config: Config,
    /// The file that was read, if any.
    pub path: Option<PathBuf>,
    /// Where scat looks for a config when none is given.
    pub default_path: Option<PathBuf>,
    pub warnings: Vec<String>,
}

/// A path for a message on stderr, with control characters made visible.
fn shown(path: &Path) -> String {
    crate::render::sanitize_str(&path.to_string_lossy())
}

pub fn default_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
    Some(base.join("scat").join("config.toml"))
}

/// Loads the config. A broken config never stops scat from printing files: problems
/// become warnings and the defaults are used.
pub fn load(explicit: Option<&Path>, disabled: bool) -> Loaded {
    let default_path = default_path();
    let mut loaded = Loaded {
        config: Config::default(),
        path: None,
        default_path: default_path.clone(),
        warnings: Vec::new(),
    };
    if disabled {
        return loaded;
    }
    let path = match explicit {
        Some(path) => path.to_path_buf(),
        None => match default_path {
            Some(path) if path.exists() => path,
            _ => return loaded,
        },
    };
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(err) => {
            loaded.warnings.push(format!(
                "config {}: {}",
                shown(&path),
                crate::util::io_message(&err)
            ));
            return loaded;
        }
    };
    match toml::from_str::<Config>(&text) {
        Ok(config) => {
            loaded.config = config;
            loaded.path = Some(path);
        }
        Err(err) => loaded.warnings.push(format!(
            "config {}: {}; using defaults",
            shown(&path),
            err.message()
        )),
    }
    loaded
}

pub fn init(explicit: Option<&Path>) -> i32 {
    let Some(path) = explicit.map(Path::to_path_buf).or_else(default_path) else {
        eprintln!("scat: cannot find a config location: HOME is not set");
        return 1;
    };
    if path.exists() {
        eprintln!("scat: {} already exists; not overwriting", shown(&path));
        return 1;
    }
    if let Some(parent) = path.parent()
        && let Err(err) = std::fs::create_dir_all(parent)
    {
        eprintln!("scat: {}: {}", shown(parent), crate::util::io_message(&err));
        return 1;
    }
    match std::fs::write(&path, TEMPLATE) {
        Ok(()) => {
            println!("{}", path.display());
            0
        }
        Err(err) => {
            eprintln!("scat: {}: {}", shown(&path), crate::util::io_message(&err));
            1
        }
    }
}

pub fn print(config: &Config) -> i32 {
    match toml::to_string(config) {
        Ok(text) => {
            print!("{text}");
            0
        }
        Err(err) => {
            eprintln!("scat: cannot serialize config: {err}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_parses() {
        toml::from_str::<Config>(TEMPLATE).expect("config template must stay valid");
    }

    #[test]
    fn later_rules_override_earlier_ones() {
        let config: Config = toml::from_str(
            r#"
            [defaults]
            tab_width = 2

            [[rule]]
            match = { mime = "text/*" }
            renderer = "text"

            [[rule]]
            match = { ext = ["rs", "toml"] }
            renderer = "code"
            line_numbers = false
            "#,
        )
        .unwrap();
        let rs = config.settings_for(Some("main.rs"), "text/plain");
        assert_eq!(rs.renderer, Renderer::Code);
        assert!(!rs.line_numbers);
        assert_eq!(rs.matched_rules, vec![1, 2]);

        let txt = config.settings_for(Some("notes.txt"), "text/plain");
        assert_eq!(txt.renderer, Renderer::Text);
        assert!(txt.line_numbers);
    }

    #[test]
    fn ext_matching_is_case_insensitive_and_supports_compound_extensions() {
        let m = Matcher {
            ext: vec!["tar.gz".into(), "MD".into()],
            ..Default::default()
        };
        assert!(m.matches(Some("a.TAR.GZ"), "application/gzip"));
        assert!(m.matches(Some("README.md"), "text/plain"));
        assert!(!m.matches(Some("md"), "text/plain"));
        assert!(!m.matches(None, "text/plain"));
    }

    #[test]
    fn unknown_keys_are_rejected() {
        assert!(toml::from_str::<Config>("[defaults]\nline_number = true").is_err());
    }
}
