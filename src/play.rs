//! `--play`: hand audio and video files to mpv.
//!
//! scat never links an audio output library. mpv runs in the foreground with the
//! terminal inherited, so its keyboard controls work, and scat waits for it.

use std::ffi::{OsStr, OsString};
use std::io::{self, BufRead, IsTerminal};
use std::process::Command;

use crate::cli::Cli;
use crate::detect::{self, SNIFF_BYTES};
use crate::input::{self, Source};
use crate::term::{self, query};
use crate::util::{io_message, which};

/// mpv exits with this status when it was stopped by a signal (Ctrl+C in the terminal).
const MPV_EXIT_SIGNAL: i32 = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaKind {
    Audio,
    Video,
}

/// The media kind for a detected MIME type, if mpv should play it.
pub fn media_kind(mime: &str) -> Option<MediaKind> {
    if mime.starts_with("audio/") {
        Some(MediaKind::Audio)
    } else if mime.starts_with("video/") {
        Some(MediaKind::Video)
    } else {
        None
    }
}

/// mpv's video output for this terminal: kitty graphics, sixel, or coloured text.
pub fn video_output(caps: Option<&query::Capabilities>) -> &'static str {
    match caps {
        Some(c) if c.kitty_graphics => "kitty",
        Some(c) if c.sixel => "sixel",
        _ => "tct",
    }
}

/// The mpv argument list (without the program name). `kind` is `None` when the type is
/// unknown (standard input): mpv then gets the video settings and plays whatever it finds.
pub fn mpv_args(kind: Option<MediaKind>, vo: &str, file: &OsStr) -> Vec<OsString> {
    let mut args: Vec<OsString> = match kind {
        Some(MediaKind::Audio) => vec!["--no-video".into(), "--term-osd-bar".into()],
        _ => vec![format!("--vo={vo}").into(), "--term-osd-bar".into()],
    };
    args.push("--".into());
    args.push(file.to_os_string());
    args
}

pub enum PlayError {
    /// mpv is not installed.
    NoMpv,
    /// The file is not audio or video; holds the detected MIME type.
    NotMedia(String),
    /// Reading the file or starting mpv failed.
    Io(io::Error),
    /// mpv exited with this non-zero status.
    Failed(i32),
    /// mpv was stopped by a signal (Ctrl+C): stop playing further files.
    Interrupted,
}

impl PlayError {
    pub fn message(&self) -> String {
        match self {
            PlayError::NoMpv => "--play needs mpv (not found on PATH)".into(),
            PlayError::NotMedia(mime) => {
                format!("not an audio or video file ({mime}); --play needs media")
            }
            PlayError::Io(err) => io_message(err),
            PlayError::Failed(code) => format!("mpv exited with status {code}"),
            PlayError::Interrupted => "interrupted".into(),
        }
    }
}

/// Detects the media kind of a source by sniffing its first bytes. Standard input is not
/// sniffed (that would consume what mpv needs) and returns `Ok(None)`.
pub fn sniff(source: &Source) -> Result<Option<MediaKind>, PlayError> {
    if matches!(source, Source::Stdin) {
        return Ok(None);
    }
    let mut input = input::open(source).map_err(PlayError::Io)?;
    let head = input.reader.fill_buf().map_err(PlayError::Io)?;
    let detected = detect::detect(&head[..head.len().min(SNIFF_BYTES)]);
    match media_kind(&detected.mime) {
        Some(kind) => Ok(Some(kind)),
        None => Err(PlayError::NotMedia(detected.mime)),
    }
}

/// Plays one source with mpv and waits for it. `vo` is the video output for video.
pub fn play(source: &Source, kind: Option<MediaKind>, vo: &str) -> Result<(), PlayError> {
    let mpv = which("mpv").ok_or(PlayError::NoMpv)?;
    let file: OsString = match source {
        Source::Stdin => "-".into(),
        Source::Path(path) => path.as_os_str().to_os_string(),
    };
    let mut command = Command::new(mpv);
    command.args(mpv_args(kind, vo, &file));

    // Like system(3): while mpv runs, Ctrl+C is mpv's to handle, not ours.
    // SAFETY: changing signal dispositions; scat is single-threaded here.
    let previous = unsafe { libc::signal(libc::SIGINT, libc::SIG_IGN) };
    let status = command.status();
    unsafe {
        libc::signal(libc::SIGINT, previous);
    }
    let status = status.map_err(PlayError::Io)?;
    match status.code() {
        Some(0) => Ok(()),
        Some(MPV_EXIT_SIGNAL) | None => Err(PlayError::Interrupted),
        Some(code) => Err(PlayError::Failed(code)),
    }
}

/// `scat --play FILE...`: plays each file in turn.
pub fn run(cli: &Cli) -> i32 {
    let mut caps: Option<Option<query::Capabilities>> = None;
    let mut status = 0;
    for source in Source::from_args(&cli.files) {
        let name = source.display_name();
        let result = sniff(&source).and_then(|kind| {
            let vo = if kind == Some(MediaKind::Audio) {
                "null"
            } else {
                let caps = caps.get_or_insert_with(query_capabilities);
                video_output(caps.as_ref())
            };
            play(&source, kind, vo)
        });
        match result {
            Ok(()) => {}
            Err(PlayError::Interrupted) => return 130,
            Err(PlayError::NoMpv) => {
                eprintln!("scat: {name}: {}", PlayError::NoMpv.message());
                return 1;
            }
            Err(err) => {
                eprintln!("scat: {name}: {}", err.message());
                status = 1;
            }
        }
    }
    status
}

fn query_capabilities() -> Option<query::Capabilities> {
    if io::stdout().is_terminal() && !term::is_basic_terminal() {
        query::query(query::DEFAULT_TIMEOUT)
            .ok()
            .filter(|c| c.answered)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_mime_types() {
        assert_eq!(media_kind("audio/mpeg"), Some(MediaKind::Audio));
        assert_eq!(media_kind("video/mp4"), Some(MediaKind::Video));
        assert_eq!(media_kind("text/plain"), None);
    }

    #[test]
    fn builds_mpv_arguments() {
        let args = mpv_args(Some(MediaKind::Audio), "null", OsStr::new("-x.mp3"));
        assert_eq!(args, ["--no-video", "--term-osd-bar", "--", "-x.mp3"]);
        let args = mpv_args(Some(MediaKind::Video), "kitty", OsStr::new("a b.mkv"));
        assert_eq!(args, ["--vo=kitty", "--term-osd-bar", "--", "a b.mkv"]);
    }

    #[test]
    fn picks_video_output() {
        let mut caps = query::Capabilities::default();
        assert_eq!(video_output(None), "tct");
        assert_eq!(video_output(Some(&caps)), "tct");
        caps.sixel = true;
        assert_eq!(video_output(Some(&caps)), "sixel");
        caps.kitty_graphics = true;
        assert_eq!(video_output(Some(&caps)), "kitty");
    }
}
