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

/// mpv options for video in the terminal, after `--vo=...`.
///
/// - `--quiet` drops the status line and `--msg-level=all=error` everything below errors.
///   mpv prints both to the same terminal as the picture, in the middle of the sixel or
///   kitty escape stream, which then ends early and the rest of the image data is shown
///   as text above and below the video.
/// - `--video-align-x/y=-1` put the picture in the top left corner. The terminal VOs draw
///   the on-screen controls into the picture itself, but take mouse positions relative to
///   the whole terminal; with a centred picture the pointer is off by the black border
///   (the controls react while the pointer is far above the seek bar).
const VIDEO_ARGS: [&str; 4] = [
    "--quiet",
    "--msg-level=all=error",
    "--video-align-x=-1",
    "--video-align-y=-1",
];

/// The mpv argument list (without the program name). `kind` is `None` when the type is
/// unknown (standard input): mpv then gets the video settings and plays whatever it finds.
pub fn mpv_args(kind: Option<MediaKind>, vo: &str, file: &OsStr) -> Vec<OsString> {
    let mut args: Vec<OsString> = match kind {
        Some(MediaKind::Audio) => vec!["--no-video".into(), "--term-osd-bar".into()],
        _ => std::iter::once(format!("--vo={vo}").into())
            .chain(VIDEO_ARGS.iter().map(OsString::from))
            .collect(),
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
                crate::report!("{name}: {}", PlayError::NoMpv.message());
                return 1;
            }
            Err(err) => {
                crate::report!("{name}: {}", err.message());
                status = 1;
            }
        }
    }
    status
}

fn query_capabilities() -> Option<query::Capabilities> {
    term::capabilities(io::stdout().is_terminal())
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
    }

    #[test]
    fn builds_video_arguments_per_output() {
        for vo in ["kitty", "sixel", "tct"] {
            let args = mpv_args(Some(MediaKind::Video), vo, OsStr::new("a b.mkv"));
            let expected = [
                format!("--vo={vo}").as_str(),
                "--quiet",
                "--msg-level=all=error",
                "--video-align-x=-1",
                "--video-align-y=-1",
                "--",
                "a b.mkv",
            ]
            .map(OsString::from);
            assert_eq!(args, expected);
            // The status line and the terminal progress bar would break the picture.
            assert!(!args.iter().any(|a| a == "--term-osd-bar"));
        }
    }

    #[test]
    fn unknown_input_gets_video_arguments() {
        let args = mpv_args(None, "sixel", OsStr::new("-"));
        assert_eq!(args[0], "--vo=sixel");
        assert!(args.iter().any(|a| a == "--quiet"));
        assert_eq!(args[args.len() - 2..], ["--", "-"]);
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
