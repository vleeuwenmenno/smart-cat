//! Paging: rendered output goes through a pager (default `less -RFX`) when it is long.
//!
//! The pager is started with an argv array, never through a shell. Its program is found
//! on `PATH` (absolute entries only) or given as an absolute path, never from the
//! current directory. In `auto` mode output is held back until it would fill the
//! screen, so short output never starts a pager at all.

use std::ffi::OsString;
use std::io::{self, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};

use crate::config::{PagingMode, Renderer};
use crate::detect::Kind;

/// Default pager: colours pass through (-R), quit at once when the output fits one
/// screen (-F), keep the output on screen after quitting (-X).
const DEFAULT_PAGER: &[&str] = &["less", "-RFX"];

/// In `auto` mode, output is held back up to this many bytes even without newlines.
const MAX_PENDING_BYTES: usize = 256 * 1024;

/// The decision made before any file is drawn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// Write straight to standard output.
    No,
    /// Page once the output is longer than the screen.
    Auto,
    /// Page all output.
    Always,
}

/// Whether to page. `all_pageable` says every file is text-like (no graphics, no
/// playback); `stdout_tty` and `render` come from the run itself.
pub fn decide(mode: PagingMode, stdout_tty: bool, render: bool, all_pageable: bool) -> Decision {
    if mode == PagingMode::Never || !stdout_tty || !render || !all_pageable {
        return Decision::No;
    }
    match mode {
        PagingMode::Always => Decision::Always,
        _ => Decision::Auto,
    }
}

/// Whether a file's rendering is plain text a pager can show. Anything that draws
/// graphics or plays media is excluded, and so is anything unknown (Auto binary files
/// become a hex dump, except types that other renderers draw).
pub fn renderer_is_pageable(
    renderer: Renderer,
    kind: &Kind,
    mime: &str,
    is_image: bool,
    is_audio: bool,
) -> bool {
    match renderer {
        Renderer::Code
        | Renderer::Text
        | Renderer::Markdown
        | Renderer::Hex
        | Renderer::Document => true,
        Renderer::Auto => {
            if is_image || is_audio {
                return false;
            }
            match kind {
                Kind::Text => !mime.starts_with("video/"),
                Kind::Binary => {
                    // Office documents render as markdown text.
                    !["image/", "audio/", "video/"]
                        .iter()
                        .any(|prefix| mime.starts_with(prefix))
                        && mime != "application/pdf"
                }
            }
        }
        _ => false,
    }
}

/// Splits a command line into words: whitespace separates, single quotes keep
/// everything, double quotes allow backslash escapes, a backslash escapes one character.
/// Returns `None` for an unclosed quote.
pub fn split_command(text: &str) -> Option<Vec<String>> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut in_word = false;
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        match c {
            c if c.is_whitespace() => {
                if in_word {
                    words.push(std::mem::take(&mut current));
                    in_word = false;
                }
            }
            '\'' => {
                in_word = true;
                loop {
                    match chars.next()? {
                        '\'' => break,
                        c => current.push(c),
                    }
                }
            }
            '"' => {
                in_word = true;
                loop {
                    match chars.next()? {
                        '"' => break,
                        '\\' => current.push(chars.next()?),
                        c => current.push(c),
                    }
                }
            }
            '\\' => {
                in_word = true;
                current.push(chars.next()?);
            }
            c => {
                in_word = true;
                current.push(c);
            }
        }
    }
    if in_word {
        words.push(current);
    }
    Some(words)
}

/// Where the pager command came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PagerCommand {
    pub argv: Vec<String>,
    /// Set through `SCAT_PAGER`: problems with it are worth a warning.
    pub explicit: bool,
}

/// Picks the pager: `SCAT_PAGER`, then `PAGER`, then `less -RFX`. An empty `SCAT_PAGER`,
/// or `PAGER` set to `cat`, turns paging off (`None`). A bare `less` gets `-RFX`.
pub fn pager_command(scat_pager: Option<&str>, pager: Option<&str>) -> Option<PagerCommand> {
    let (text, explicit) = match (scat_pager, pager) {
        (Some(text), _) => (Some(text), true),
        (None, Some(text)) if !text.trim().is_empty() => (Some(text), false),
        _ => (None, false),
    };
    let mut argv = match text {
        Some(text) => {
            let argv = split_command(text)?;
            if argv.is_empty() {
                return None;
            }
            argv
        }
        None => DEFAULT_PAGER.iter().map(|s| (*s).to_string()).collect(),
    };
    let program = std::path::Path::new(&argv[0])
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    if program == "cat" || program == "scat" {
        return None;
    }
    if program == "less" && argv.len() == 1 {
        argv.push("-RFX".into());
    }
    Some(PagerCommand { argv, explicit })
}

/// Finds the pager program without ever looking in the current directory.
pub fn resolve_program(name: &str) -> Option<PathBuf> {
    if name.contains('/') {
        let path = PathBuf::from(name);
        let ok = path.is_absolute() && {
            use std::os::unix::fs::PermissionsExt;
            path.metadata()
                .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        };
        return ok.then_some(path);
    }
    crate::util::which(name)
}

/// What `Output` needs to start a pager.
pub struct Plan {
    pub program: PathBuf,
    pub args: Vec<OsString>,
    pub decision: Decision,
    /// Terminal height in rows.
    pub rows: usize,
}

/// Builds the plan from the environment, or `None` when paging is off or the pager
/// cannot be found.
pub fn plan(decision: Decision) -> Option<Plan> {
    if decision == Decision::No {
        return None;
    }
    let command = pager_command(
        std::env::var("SCAT_PAGER").ok().as_deref(),
        std::env::var("PAGER").ok().as_deref(),
    )?;
    let Some(program) = resolve_program(&command.argv[0]) else {
        if command.explicit || decision == Decision::Always {
            eprintln!(
                "scat: pager '{}' not found; printing without a pager",
                crate::render::sanitize_str(&command.argv[0])
            );
        }
        return None;
    };
    Some(Plan {
        program,
        args: command.argv[1..].iter().map(OsString::from).collect(),
        decision,
        rows: terminal_rows(),
    })
}

fn terminal_rows() -> usize {
    if let Ok(ws) = rustix::termios::tcgetwinsize(io::stdout())
        && ws.ws_row > 0
    {
        return ws.ws_row as usize;
    }
    std::env::var("LINES")
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .filter(|&rows| rows > 0)
        .unwrap_or(24)
}

enum State {
    Direct,
    /// Held back until the output is long enough to need the pager.
    Pending {
        buf: Vec<u8>,
        newlines: usize,
        limit: usize,
    },
    Pager {
        child: Child,
        stdin: Option<ChildStdin>,
    },
}

/// Standard output, optionally through a pager.
pub struct Output<W: Write> {
    direct: W,
    state: State,
    plan: Option<Plan>,
}

impl<W: Write> Output<W> {
    pub fn new(direct: W, plan: Option<Plan>) -> Self {
        let state = match &plan {
            None => State::Direct,
            Some(plan) => State::Pending {
                buf: Vec::new(),
                newlines: 0,
                limit: match plan.decision {
                    Decision::Always => 0,
                    _ => plan.rows.saturating_sub(1).max(1),
                },
            },
        };
        Self {
            direct,
            state,
            plan,
        }
    }

    /// True when a pager is (or may get) in charge of the output.
    pub fn is_paged(&self) -> bool {
        self.plan.is_some()
    }

    /// Starts the pager and hands it what was held back. Falls back to plain output when
    /// the pager cannot start.
    fn start_pager(&mut self, held: Vec<u8>) -> io::Result<()> {
        let Some(plan) = &self.plan else {
            self.state = State::Direct;
            return self.direct.write_all(&held);
        };
        let spawned = Command::new(&plan.program)
            .args(&plan.args)
            .stdin(Stdio::piped())
            .spawn();
        match spawned {
            Ok(mut child) => {
                let stdin = child.stdin.take();
                // A pager that quits early closes the pipe: that must be an error to
                // handle, not a signal that kills scat. Set after the spawn so the
                // pager does not inherit the setting.
                // SAFETY: changing a signal disposition; no handler runs our code.
                unsafe {
                    libc::signal(libc::SIGPIPE, libc::SIG_IGN);
                }
                self.state = State::Pager { child, stdin };
                self.write_all(&held)
            }
            Err(err) => {
                eprintln!(
                    "scat: pager {}: {}; printing without a pager",
                    crate::render::sanitize_str(&plan.program.to_string_lossy()),
                    crate::util::io_message(&err)
                );
                self.state = State::Direct;
                self.direct.write_all(&held)
            }
        }
    }

    /// Ends the output: prints what was held back (it was short) or closes the pager
    /// and waits for it.
    pub fn finish(&mut self) -> io::Result<()> {
        match std::mem::replace(&mut self.state, State::Direct) {
            State::Direct => self.direct.flush(),
            State::Pending { buf, .. } => {
                self.direct.write_all(&buf)?;
                self.direct.flush()
            }
            State::Pager { mut child, stdin } => {
                drop(stdin);
                child.wait().map(|_| ())
            }
        }
    }
}

impl<W: Write> Write for Output<W> {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        match &mut self.state {
            State::Direct => self.direct.write(data),
            State::Pending {
                buf,
                newlines,
                limit,
            } => {
                buf.extend_from_slice(data);
                *newlines += data.iter().filter(|&&b| b == b'\n').count();
                if *newlines >= *limit || buf.len() > MAX_PENDING_BYTES {
                    let held = std::mem::take(buf);
                    self.start_pager(held)?;
                }
                Ok(data.len())
            }
            State::Pager { stdin, .. } => match stdin {
                Some(stdin) => stdin.write(data),
                None => Err(io::ErrorKind::BrokenPipe.into()),
            },
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match &mut self.state {
            State::Direct => self.direct.flush(),
            State::Pending { .. } => Ok(()),
            State::Pager { stdin, .. } => stdin.as_mut().map_or(Ok(()), Write::flush),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(text: &str) -> Vec<String> {
        split_command(text).unwrap()
    }

    #[test]
    fn splits_like_a_shell_without_one() {
        assert_eq!(words("less -R  -F"), ["less", "-R", "-F"]);
        assert_eq!(
            words(r#"my\ pager "a b" 'c "d"'"#),
            ["my pager", "a b", "c \"d\""]
        );
        assert_eq!(words(r#""a\"b""#), ["a\"b"]);
        assert_eq!(words("''"), [""]);
        assert!(words("   ").is_empty());
        assert!(split_command("less 'x").is_none());
        // No expansion: $VAR and ; stay literal.
        assert_eq!(words("less $HOME;ls"), ["less", "$HOME;ls"]);
    }

    #[test]
    fn picks_the_pager() {
        let argv = |c: Option<PagerCommand>| c.map(|c| c.argv);
        assert_eq!(
            argv(pager_command(None, None)),
            Some(vec!["less".into(), "-RFX".into()])
        );
        assert_eq!(
            argv(pager_command(None, Some("less"))),
            Some(vec!["less".into(), "-RFX".into()])
        );
        assert_eq!(
            argv(pager_command(None, Some("most -s"))),
            Some(vec!["most".into(), "-s".into()])
        );
        assert_eq!(
            argv(pager_command(Some("bat -p"), Some("most"))),
            Some(vec!["bat".into(), "-p".into()])
        );
        assert!(pager_command(Some(""), Some("most")).is_none());
        assert!(pager_command(None, Some("cat")).is_none());
        assert!(pager_command(None, Some("/usr/bin/scat")).is_none());
        // Empty PAGER falls back to the default.
        assert!(pager_command(None, Some("")).is_some());
        assert!(pager_command(Some("less 'x"), None).is_none());
        assert!(pager_command(Some("bat"), None).unwrap().explicit);
        assert!(!pager_command(None, Some("most")).unwrap().explicit);
    }

    #[test]
    fn paging_decision() {
        use PagingMode::*;
        assert_eq!(decide(Auto, true, true, true), Decision::Auto);
        assert_eq!(decide(Always, true, true, true), Decision::Always);
        assert_eq!(decide(Never, true, true, true), Decision::No);
        // Not a terminal, not rendering (--raw, cat flags), or graphics involved.
        assert_eq!(decide(Always, false, true, true), Decision::No);
        assert_eq!(decide(Always, true, false, true), Decision::No);
        assert_eq!(decide(Always, true, true, false), Decision::No);
    }

    #[test]
    fn only_text_like_files_are_pageable() {
        let text = Kind::Text;
        let bin = Kind::Binary;
        assert!(renderer_is_pageable(
            Renderer::Auto,
            &text,
            "text/plain",
            false,
            false
        ));
        assert!(renderer_is_pageable(
            Renderer::Auto,
            &bin,
            "application/x-executable",
            false,
            false
        ));
        assert!(renderer_is_pageable(
            Renderer::Markdown,
            &text,
            "text/plain",
            false,
            false
        ));
        assert!(renderer_is_pageable(
            Renderer::Hex,
            &bin,
            "image/png",
            true,
            false
        ));
        assert!(!renderer_is_pageable(
            Renderer::Auto,
            &bin,
            "image/png",
            true,
            false
        ));
        assert!(!renderer_is_pageable(
            Renderer::Auto,
            &text,
            "image/svg+xml",
            true,
            false
        ));
        assert!(!renderer_is_pageable(
            Renderer::Auto,
            &bin,
            "audio/flac",
            false,
            true
        ));
        assert!(!renderer_is_pageable(
            Renderer::Auto,
            &bin,
            "video/mp4",
            false,
            false
        ));
        assert!(!renderer_is_pageable(
            Renderer::Auto,
            &bin,
            "application/pdf",
            false,
            false
        ));
        assert!(!renderer_is_pageable(
            Renderer::Image,
            &bin,
            "image/png",
            true,
            false
        ));
        assert!(!renderer_is_pageable(
            Renderer::Raw,
            &text,
            "text/plain",
            false,
            false
        ));
    }

    #[test]
    fn short_output_never_starts_a_pager() {
        let plan = Plan {
            program: "/nonexistent/pager".into(),
            args: vec![],
            decision: Decision::Auto,
            rows: 10,
        };
        let mut out = Output::new(Vec::new(), Some(plan));
        out.write_all(b"one\ntwo\n").unwrap();
        out.flush().unwrap();
        assert!(out.direct.is_empty(), "held back until finish");
        out.finish().unwrap();
        assert_eq!(out.direct, b"one\ntwo\n");
    }

    #[test]
    fn long_output_with_a_broken_pager_falls_back_to_plain_output() {
        let plan = Plan {
            program: "/nonexistent/pager".into(),
            args: vec![],
            decision: Decision::Auto,
            rows: 3,
        };
        let mut out = Output::new(Vec::new(), Some(plan));
        out.write_all(b"1\n2\n3\n").unwrap();
        out.write_all(b"4\n").unwrap();
        out.finish().unwrap();
        assert_eq!(out.direct, b"1\n2\n3\n4\n");
    }
}
