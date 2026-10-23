//! Video info view. The ffmpeg-based tests skip themselves when ffmpeg is missing.

use std::path::PathBuf;
use std::process::{Command, Output};

const SCAT: &str = env!("CARGO_BIN_EXE_scat");

fn dir() -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("video");
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn scat(args: &[&str], cwd: &std::path::Path) -> Output {
    Command::new(SCAT)
        .args(args)
        .current_dir(cwd)
        .env_remove("SCAT_CONFIG")
        .env("HOME", cwd)
        .env_remove("XDG_CONFIG_HOME")
        .output()
        .unwrap()
}

fn make_video(dir: &std::path::Path, name: &str) -> bool {
    let ok = Command::new("ffmpeg")
        .args(["-v", "error", "-y", "-f", "lavfi", "-i"])
        .arg("testsrc=duration=1:size=320x240:rate=25")
        .args(["-f", "lavfi", "-i", "sine=duration=1", "-shortest"])
        .arg(name)
        .current_dir(dir)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !ok {
        eprintln!("skipping: ffmpeg not available");
    }
    ok
}

#[test]
fn info_view_shows_stream_details() {
    let dir = dir();
    if !make_video(&dir, "out.mp4") {
        return;
    }
    let out = scat(&["-f", "--color=never", "out.mp4"], &dir);
    assert!(out.status.success());
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.starts_with("out.mp4\n"), "{text}");
    assert!(text.contains("320x240"), "{text}");
    assert!(text.contains("25 fps"), "{text}");
    assert!(text.contains("audio"), "{text}");
}

#[test]
fn odd_file_names_reach_ffprobe_intact() {
    let dir = dir();
    if !make_video(&dir, "src.mp4") {
        return;
    }
    for name in ["a b.mp4", "c:d.mp4", "-e.mp4", "http:x.mp4"] {
        std::fs::copy(dir.join("src.mp4"), dir.join(name)).unwrap();
        let out = scat(&["-f", "--color=never", "--", name], &dir);
        let text = String::from_utf8(out.stdout).unwrap();
        assert!(text.contains("320x240"), "{name}: {text}");
    }
}

#[test]
fn piped_output_is_the_raw_file() {
    let dir = dir();
    let mut bytes = b"\0\0\0\x18ftypmp42".to_vec();
    bytes.resize(200, 0);
    std::fs::write(dir.join("fake.mp4"), &bytes).unwrap();
    let out = scat(&["fake.mp4"], &dir);
    assert_eq!(out.stdout, bytes);
}

#[test]
fn unreadable_video_still_shows_a_table() {
    let dir = dir();
    let mut bytes = b"\0\0\0\x18ftypmp42".to_vec();
    bytes.resize(200, 0);
    std::fs::write(dir.join("broken.mp4"), &bytes).unwrap();
    let out = scat(&["-f", "--color=never", "broken.mp4"], &dir);
    assert!(out.status.success());
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.contains("video/mp4"), "{text}");
    assert!(text.contains("200 B"), "{text}");
    assert!(
        text.contains("ffprobe could not read this file")
            || text.contains("install ffmpeg (ffprobe)"),
        "{text}"
    );
}

#[test]
fn stdin_gets_a_notice_and_status_zero() {
    let dir = dir();
    let mut bytes = b"\0\0\0\x18ftypmp42".to_vec();
    bytes.resize(200, 0);
    std::fs::write(dir.join("fake.mp4"), &bytes).unwrap();
    let out = Command::new(SCAT)
        .args(["-f", "--color=never"])
        .stdin(std::fs::File::open(dir.join("fake.mp4")).unwrap())
        .current_dir(&dir)
        .env("HOME", &dir)
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(out.stdout.is_empty());
    assert_eq!(
        String::from_utf8_lossy(&out.stderr),
        "scat: -: video info needs a file (use --raw for bytes)\n"
    );
}
