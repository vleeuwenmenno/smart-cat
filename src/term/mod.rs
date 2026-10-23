//! Terminal environment: colour support, locale and capability queries.

pub mod cache;
pub mod query;

use std::env;

use crate::config::ColorWhen;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ColorDepth {
    Ansi16,
    Ansi256,
    TrueColor,
}

impl ColorDepth {
    pub fn name(self) -> &'static str {
        match self {
            ColorDepth::Ansi16 => "16 colours",
            ColorDepth::Ansi256 => "256 colours",
            ColorDepth::TrueColor => "24-bit",
        }
    }
}

fn var(name: &str) -> Option<String> {
    env::var(name).ok().filter(|v| !v.is_empty())
}

pub fn term_name() -> Option<String> {
    var("TERM")
}

/// Whether this process is in the foreground process group of the terminal `fd`.
/// A background job (`scat file &`) that changes the terminal mode is stopped with
/// SIGTTOU, and one that reads the terminal (a pager) with SIGTTIN; cat would just print.
pub fn in_foreground<Fd: std::os::fd::AsFd>(fd: Fd) -> bool {
    rustix::termios::tcgetpgrp(fd).is_ok_and(|pgrp| pgrp == rustix::process::getpgrp())
}

/// Terminals that never answer queries and support no graphics.
pub fn is_basic_terminal() -> bool {
    match term_name().as_deref() {
        None => true,
        Some(t) => t == "dumb" || t == "linux" || t.starts_with("vt") || t == "cons25",
    }
}

/// The terminal's capabilities for a run that prints to it: from the cache when a
/// fresh entry exists, otherwise by querying (and caching the result, including "did
/// not answer"). `None` when stdout is not a terminal, the terminal is a basic one,
/// the query could not run, or the terminal did not answer.
pub fn capabilities(stdout_tty: bool) -> Option<query::Capabilities> {
    if !stdout_tty || is_basic_terminal() {
        return None;
    }
    let cache = cache::dir().zip(cache::Key::current());
    if let Some((dir, key)) = &cache
        && let Some(entry) = cache::load(dir, key, std::time::SystemTime::now())
    {
        return entry.caps.answered.then_some(entry.caps);
    }
    // A query that could not run (background job, typeahead) is not cached.
    let caps = query::query(query::DEFAULT_TIMEOUT).ok()?;
    if let Some((dir, key)) = &cache {
        let _ = cache::store(dir, key, &caps, std::time::SystemTime::now());
    }
    caps.answered.then_some(caps)
}

pub fn color_depth() -> ColorDepth {
    let colorterm = var("COLORTERM").unwrap_or_default().to_ascii_lowercase();
    let term = term_name().unwrap_or_default();
    if term == "linux" || term.starts_with("vt") {
        return ColorDepth::Ansi16;
    }
    if colorterm == "truecolor" || colorterm == "24bit" || term.ends_with("-direct") {
        return ColorDepth::TrueColor;
    }
    if term.contains("256color")
        || term == "xterm-kitty"
        || term == "foot"
        || term == "xterm-ghostty"
    {
        return ColorDepth::Ansi256;
    }
    ColorDepth::Ansi16
}

/// Resolves --color / config against NO_COLOR, CLICOLOR_FORCE and TERM=dumb.
pub fn colors_enabled(when: ColorWhen, stdout_is_tty: bool) -> bool {
    match when {
        ColorWhen::Always => true,
        ColorWhen::Never => false,
        ColorWhen::Auto => {
            if var("NO_COLOR").is_some() {
                return false;
            }
            if var("CLICOLOR_FORCE").is_some_and(|v| v != "0") {
                return true;
            }
            if var("CLICOLOR").as_deref() == Some("0") {
                return false;
            }
            stdout_is_tty && term_name().as_deref() != Some("dumb")
        }
    }
}

pub fn utf8_locale() -> bool {
    ["LC_ALL", "LC_CTYPE", "LANG"]
        .iter()
        .find_map(|name| var(name))
        .map(|v| {
            let v = v.to_ascii_lowercase();
            v.contains("utf-8") || v.contains("utf8")
        })
        .unwrap_or(false)
}

/// The multiplexer scat is running inside, if any.
pub fn multiplexer() -> Option<&'static str> {
    if var("TMUX").is_some() {
        Some("tmux")
    } else if var("ZELLIJ").is_some() {
        Some("zellij")
    } else if var("STY").is_some() {
        Some("screen")
    } else {
        None
    }
}

/// Environment variables that hint at the terminal, for --diagnose.
pub const HINT_VARS: &[&str] = &[
    "TERM",
    "TERM_PROGRAM",
    "TERM_PROGRAM_VERSION",
    "COLORTERM",
    "KITTY_WINDOW_ID",
    "GHOSTTY_RESOURCES_DIR",
    "WEZTERM_EXECUTABLE",
    "KONSOLE_VERSION",
    "VTE_VERSION",
    "ALACRITTY_WINDOW_ID",
    "TMUX",
    "ZELLIJ",
    "SSH_TTY",
    "NO_COLOR",
    "CLICOLOR_FORCE",
];

pub fn hint(name: &str) -> Option<String> {
    var(name)
}
