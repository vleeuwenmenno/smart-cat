//! Subtitle view through the binary, with files written by the test.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const SCAT: &str = env!("CARGO_BIN_EXE_scat");

fn dir(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("subtitles")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, bytes).unwrap();
    path
}

fn scat(args: &[&str], file: &Path) -> Output {
    Command::new(SCAT)
        .args(args)
        .arg(file)
        .env("COLUMNS", "100")
        .env("NO_COLOR", "1")
        .env("SCAT_CONFIG", "/nonexistent")
        .output()
        .unwrap()
}

fn view(file: &Path) -> String {
    let out = scat(&["--force-render", "--plain", "--color=never"], file);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

const SRT: &str = "1\n00:00:01,000 --> 00:00:03,500\nHello <i>world</i>\nsecond line\n\n\
2\n00:00:03,000 --> 00:00:04,000\nOverlaps the first &amp; is fast: this text is far too long for one second\n\n\
3\n00:00:20,000 --> 00:00:35,000\n{\\an8}Very long cue\n";

#[test]
fn srt_overview_cues_and_warnings() {
    let d = dir("srt");
    let file = write(&d, "movie.en.srt", SRT.as_bytes());
    let text = view(&file);
    assert!(text.contains("SubRip"), "{text}");
    assert!(text.contains("cues          3"), "{text}");
    assert!(text.contains("00:00:35.000"), "{text}");
    assert!(text.contains("en (from the file name)"), "{text}");
    assert!(text.contains("overlapping   1 cue "), "{text}");
    assert!(text.contains("fast reading  1 cue "), "{text}");
    assert!(text.contains("too long      1 cue "), "{text}");
    assert!(
        text.contains("00:00:01.000 → 00:00:03.500  Hello world"),
        "{text}"
    );
    // Continuation lines sit under the text.
    assert!(text.contains(&format!("\n{:29}second line", "")), "{text}");
    assert!(text.contains("& is fast"), "{text}");
    assert!(!text.contains("{\\an8}") && !text.contains("<i>"), "{text}");
}

#[test]
fn cue_numbers_with_dash_n() {
    let d = dir("numbers");
    let file = write(&d, "a.srt", SRT.as_bytes());
    let out = scat(&["--force-render", "--color=never", "-n"], &file);
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.contains("   1 │ 00:00:01.000"), "{text}");
    assert!(text.contains("   3 │ 00:00:20.000"), "{text}");
}

#[test]
fn vtt_tags_settings_and_skipped_blocks() {
    let d = dir("vtt");
    let vtt = "WEBVTT\nLanguage: nl\n\nNOTE a comment\nwith two lines\n\nSTYLE\n::cue { color: red }\n\n\
intro\n00:01.000 --> 00:03.000 align:start line:90%\n<v Bob><i>Hi</i> <c.yellow>there</c> <00:02.000>you\n\n\
00:04.000 --> 00:05.000\nBye\n";
    let file = write(&d, "talk.vtt", vtt.as_bytes());
    let text = view(&file);
    assert!(text.contains("WebVTT"), "{text}");
    assert!(text.contains("language  nl"), "{text}");
    assert!(text.contains("[align:start line:90%]"), "{text}");
    assert!(text.contains("Hi there you"), "{text}");
    assert!(
        !text.contains("a comment") && !text.contains("color: red"),
        "{text}"
    );
    assert!(text.contains("cues      2"), "{text}");
    // Italics become SGR italic when colours are on.
    let out = scat(&["--force-render", "--plain", "--color=always"], &file);
    let coloured = String::from_utf8(out.stdout).unwrap();
    assert!(coloured.contains("\x1b[3mHi\x1b[23m"), "{coloured:?}");
}

#[test]
fn ass_styles_title_and_overrides() {
    let d = dir("ass");
    let ass = "[Script Info]\nTitle: Test Show\nScriptType: v4.00+\n\n[V4+ Styles]\n\
Format: Name, Fontname\nStyle: Default,Arial\nStyle: Sign,Arial\n\n[Events]\n\
Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n\
Dialogue: 0,0:00:01.00,0:00:03.00,Default,,0,0,0,,{\\i1}Hello{\\i0}\\Nthere, friend\n\
Dialogue: 0,0:00:04.00,0:00:06.00,Sign,Mia,0,0,0,,{\\pos(1,2)}Sign text\n\
Comment: 0,0:00:07.00,0:00:08.00,Default,,0,0,0,,ignored\n";
    let file = write(&d, "show.ass", ass.as_bytes());
    let text = view(&file);
    assert!(text.contains("Advanced SubStation"), "{text}");
    assert!(text.contains("title   Test Show"), "{text}");
    assert!(text.contains("styles  2"), "{text}");
    assert!(text.contains("cues    2"), "{text}");
    assert!(
        text.contains("00:00:01.000 → 00:00:03.000  Hello"),
        "{text}"
    );
    assert!(text.contains("there, friend"), "{text}");
    assert!(text.contains("[Sign / Mia] Sign text"), "{text}");
    assert!(
        !text.contains("[Default]") && !text.contains("ignored"),
        "{text}"
    );
}

#[test]
fn latin1_and_utf16() {
    let d = dir("encodings");
    let latin = write(
        &d,
        "l.srt",
        b"1\n00:00:01,000 --> 00:00:02,000\ncaf\xE9 cr\xE8me\n",
    );
    let text = view(&latin);
    assert!(text.contains("café crème"), "{text}");
    assert!(text.contains("Windows-1252"), "{text}");

    let mut utf16 = vec![0xFF, 0xFE];
    for unit in "1\n00:00:01,000 --> 00:00:02,000\nhéllo\n".encode_utf16() {
        utf16.extend(unit.to_le_bytes());
    }
    let wide = write(&d, "w.srt", &utf16);
    let text = view(&wide);
    assert!(text.contains("héllo"), "{text}");
    assert!(text.contains("UTF-16 LE"), "{text}");
}

#[test]
fn cue_cap_footer() {
    let d = dir("cap");
    let mut srt = String::new();
    for i in 0..600 {
        srt += &format!(
            "{}\n00:{:02}:{:02},000 --> 00:{:02}:{:02},500\ncue {i}x\n\n",
            i + 1,
            i / 60,
            i % 60,
            i / 60,
            i % 60
        );
    }
    let file = write(&d, "big.srt", srt.as_bytes());
    let text = view(&file);
    assert!(text.contains("cues    600"), "{text}");
    assert!(text.contains("… 100 more cues"), "{text}");
    assert!(
        text.contains("cue 499x") && !text.contains("cue 500x"),
        "{text}"
    );
}

#[test]
fn piped_bytes_unchanged() {
    let d = dir("piped");
    let file = write(&d, "p.srt", SRT.as_bytes());
    let out = scat(&[], &file);
    assert_eq!(out.stdout, SRT.as_bytes());
}

#[test]
fn broken_file_falls_back_to_text() {
    let d = dir("broken");
    let file = write(&d, "bad.srt", b"this is not\na subtitle file\n");
    let out = scat(&["--force-render", "--plain", "--color=never"], &file);
    assert!(out.status.success());
    let text = String::from_utf8(out.stdout).unwrap();
    assert_eq!(text, "this is not\na subtitle file\n");
    let err = String::from_utf8(out.stderr).unwrap();
    assert!(
        err.contains("bad.srt") && err.contains("showing it as text"),
        "{err}"
    );
}

#[test]
fn renderer_code_shows_the_source() {
    let d = dir("code");
    let file = write(&d, "c.srt", SRT.as_bytes());
    let config = write(
        &d,
        "config.toml",
        b"[[rule]]\nmatch = { ext = \"srt\" }\nrenderer = \"code\"\n",
    );
    let out = Command::new(SCAT)
        .args(["--force-render", "--plain", "--color=never"])
        .arg(&file)
        .env("COLUMNS", "100")
        .env("SCAT_CONFIG", &config)
        .output()
        .unwrap();
    assert_eq!(String::from_utf8(out.stdout).unwrap(), SRT);
}
