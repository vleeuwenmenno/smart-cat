//! Audio info view: a hand-written WAV file renders a table and a waveform, and piped
//! output stays the file's bytes.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const SCAT: &str = env!("CARGO_BIN_EXE_scat");

/// A 16-bit mono PCM WAV: `seconds` of a 440 Hz sine whose loudness ramps up, followed
/// by a LIST/INFO chunk with a title and artist.
fn wav(seconds: u32) -> Vec<u8> {
    let rate = 8000u32;
    let frames = rate * seconds;
    let mut pcm = Vec::with_capacity(frames as usize * 2);
    for i in 0..frames {
        let t = i as f64 / f64::from(rate);
        let loudness = f64::from(i) / f64::from(frames);
        let sample = (t * 440.0 * std::f64::consts::TAU).sin() * loudness * 30000.0;
        pcm.extend_from_slice(&(sample as i16).to_le_bytes());
    }

    let mut info = b"INFO".to_vec();
    for (id, text) in [(b"INAM", "Test \x1b]0;pwned\x07Tone"), (b"IART", "scat")] {
        let mut value = text.as_bytes().to_vec();
        value.push(0);
        if value.len() % 2 == 1 {
            value.push(0);
        }
        info.extend_from_slice(id);
        info.extend_from_slice(&(value.len() as u32).to_le_bytes());
        info.extend_from_slice(&value);
    }

    let mut body = b"WAVE".to_vec();
    body.extend_from_slice(b"fmt ");
    body.extend_from_slice(&16u32.to_le_bytes());
    body.extend_from_slice(&1u16.to_le_bytes()); // PCM
    body.extend_from_slice(&1u16.to_le_bytes()); // mono
    body.extend_from_slice(&rate.to_le_bytes());
    body.extend_from_slice(&(rate * 2).to_le_bytes()); // byte rate
    body.extend_from_slice(&2u16.to_le_bytes()); // block align
    body.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    body.extend_from_slice(b"data");
    body.extend_from_slice(&(pcm.len() as u32).to_le_bytes());
    body.extend_from_slice(&pcm);
    body.extend_from_slice(b"LIST");
    body.extend_from_slice(&(info.len() as u32).to_le_bytes());
    body.extend_from_slice(&info);

    let mut file = b"RIFF".to_vec();
    file.extend_from_slice(&(body.len() as u32).to_le_bytes());
    file.extend_from_slice(&body);
    file
}

fn fixture(name: &str, data: &[u8]) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("audio");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    std::fs::write(&path, data).unwrap();
    path
}

fn scat(args: &[&str], dir: &Path, locale: &str) -> Output {
    Command::new(SCAT)
        .args(args)
        .current_dir(dir)
        .env("HOME", dir)
        .env_remove("SCAT_CONFIG")
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("SCAT_COLOR")
        .env("LC_ALL", locale)
        .env("COLUMNS", "40")
        .output()
        .unwrap()
}

#[test]
fn renders_table_and_waveform() {
    let path = fixture("tone.wav", &wav(2));
    let dir = path.parent().unwrap();
    let out = scat(&["-f", "--color=never", "tone.wav"], dir, "C.UTF-8");
    assert!(out.status.success(), "{out:?}");
    let text = String::from_utf8(out.stdout).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines[0], "tone.wav");
    for row in [
        "duration     0:02",
        "format       WAV",
        "sample rate  8 kHz",
        "channels     1 (mono)",
        "bit depth    16 bit",
        "artist       scat",
    ] {
        assert!(
            lines.iter().any(|l| l.trim() == row),
            "missing {row:?} in\n{text}"
        );
    }
    // The title's escape sequence is shown, not executed.
    assert!(text.contains("Test ^[]0;pwned^GTone"), "{text}");
    assert!(!text.contains('\x1b'));

    let wave: Vec<&str> = lines
        .iter()
        .copied()
        .filter(|l| !l.is_empty() && l.chars().all(|c| ('\u{2800}'..='\u{28ff}').contains(&c)))
        .collect();
    assert_eq!(wave.len(), 4, "{text}");
    assert!(wave.iter().all(|l| l.chars().count() == 40));
    // Loudness ramps up: the end of the outer rows is drawn, the start is not.
    assert!(wave[0].starts_with('\u{2800}'));
    assert!(wave[0].ends_with('\u{28ff}'));
}

#[test]
fn ascii_waveform_without_utf8() {
    let path = fixture("tone-ascii.wav", &wav(1));
    let dir = path.parent().unwrap();
    let out = scat(&["-f", "--color=never", "tone-ascii.wav"], dir, "C");
    assert!(out.status.success());
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.is_ascii(), "{text}");
    assert!(text.contains('#'), "{text}");
}

#[test]
fn piped_output_is_the_file() {
    let data = wav(1);
    let path = fixture("tone-pipe.wav", &data);
    let dir = path.parent().unwrap();
    let out = scat(&["tone-pipe.wav"], dir, "C.UTF-8");
    assert!(out.status.success());
    assert_eq!(out.stdout, data);
}

#[test]
fn reads_audio_from_stdin() {
    let data = wav(1);
    let path = fixture("tone-stdin.wav", &data);
    let dir = path.parent().unwrap();
    let out = Command::new(SCAT)
        .args(["-f", "--color=never"])
        .stdin(std::fs::File::open(&path).unwrap())
        .current_dir(dir)
        .env("HOME", dir)
        .env("LC_ALL", "C.UTF-8")
        .output()
        .unwrap();
    assert!(out.status.success());
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.starts_with("-\n"), "{text}");
    assert!(text.contains("WAV"), "{text}");
}

#[test]
fn malformed_audio_does_not_crash() {
    let mut data = wav(1);
    data.truncate(300);
    let truncated = fixture("truncated.wav", &data);
    let dir = truncated.parent().unwrap();
    let out = scat(&["-f", "--color=never", "truncated.wav"], dir, "C.UTF-8");
    assert!(out.status.success(), "{out:?}");
    assert!(String::from_utf8_lossy(&out.stdout).starts_with("truncated.wav"));

    // An MPEG frame sync followed by junk: detected as audio/mpeg, unreadable.
    let mut junk = vec![0xff, 0xfb, 0x90, 0x64];
    junk.extend((0..4096u32).map(|i| (i.wrapping_mul(2654435761) >> 13) as u8));
    fixture("junk.mp3", &junk);
    let out = scat(&["-f", "--color=never", "junk.mp3"], dir, "C.UTF-8");
    assert!(out.status.code().is_some(), "{out:?}");
}

#[test]
fn play_rejects_non_media() {
    let path = fixture("notes.txt", b"hello\n");
    let dir = path.parent().unwrap();
    let out = Command::new(SCAT)
        .args(["--play", "notes.txt"])
        .current_dir(dir)
        .env("PATH", dir) // no mpv here either way
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.starts_with("scat: notes.txt: "), "{err}");
}

#[test]
fn play_without_mpv() {
    let path = fixture("tone-play.wav", &wav(1));
    let dir = path.parent().unwrap();
    let out = Command::new(SCAT)
        .args(["--play", "tone-play.wav"])
        .current_dir(dir)
        .env("PATH", dir)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(
        String::from_utf8_lossy(&out.stderr),
        "scat: tone-play.wav: --play needs mpv (not found on PATH)\n"
    );
}
