//! Image rendering through the binary: forced text output, captions, the cat contract.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use image::{Rgba, RgbaImage};

const SCAT: &str = env!("CARGO_BIN_EXE_scat");

fn fixtures() -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("images");
    std::fs::create_dir_all(&dir).unwrap();
    let img = RgbaImage::from_fn(40, 20, |x, y| {
        if x < 20 {
            Rgba([255, 0, 0, 255])
        } else if y < 10 {
            Rgba([0, 0, 255, 255])
        } else {
            Rgba([0, 0, 0, 0])
        }
    });
    img.save(dir.join("x.png")).unwrap();
    std::fs::write(dir.join("broken.png"), b"\x89PNG\r\n\x1a\nnot really").unwrap();
    std::fs::write(
        dir.join("shape.svg"),
        br#"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="10"><rect width="20" height="10" fill="lime"/></svg>"#,
    )
    .unwrap();
    std::fs::write(dir.join("notes.txt"), b"hello\n").unwrap();
    dir
}

fn run(args: &[&str], dir: &Path) -> Output {
    Command::new(SCAT)
        .args(args)
        .current_dir(dir)
        .env_remove("SCAT_CONFIG")
        .env_remove("SCAT_PROTOCOL")
        .env("HOME", dir)
        .env_remove("XDG_CONFIG_HOME")
        .env("COLUMNS", "40")
        .env("LINES", "30")
        .env("COLORTERM", "truecolor")
        .env("LC_ALL", "C.UTF-8")
        .output()
        .unwrap()
}

/// Every escape sequence in `text` is an SGR (`ESC [ ... m`).
fn only_sgr(text: &str) -> bool {
    let mut rest = text;
    while let Some(pos) = rest.find('\x1b') {
        let seq = &rest[pos + 1..];
        let Some(body) = seq.strip_prefix('[') else {
            return false;
        };
        let end = body
            .find(|c: char| !(c.is_ascii_digit() || c == ';'))
            .unwrap_or(body.len());
        if !body[end..].starts_with('m') {
            return false;
        }
        rest = &body[end + 1..];
    }
    true
}

#[test]
fn blocks_draw_half_block_cells_and_a_caption() {
    let dir = fixtures();
    let out = run(
        &["-f", "--protocol=blocks", "--color=always", "x.png"],
        &dir,
    );
    assert_eq!(out.status.code(), Some(0));
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.contains('▀'), "{text:?}");
    assert!(
        text.contains("38;2;255;0;0"),
        "red as 24-bit colour: {text:?}"
    );
    assert!(only_sgr(&text), "{text:?}");
    let caption = text.lines().last().unwrap();
    let expected = format!(
        "\x1b[2mx.png  40x20 PNG, {}\x1b[22m",
        size_of(&dir, "x.png")
    );
    assert_eq!(caption, expected);
    // A small 40x20 image is not blown up to the 40 columns: text output enlarges it
    // to one sub-pixel per pixel at most, here capped at 32 columns. A 2:1 image on
    // 1:2 cells is then 8 rows, plus the caption.
    assert_eq!(text.lines().count(), 9);
    assert_eq!(text.lines().next().unwrap().matches('▀').count(), 32);
}

fn size_of(dir: &Path, name: &str) -> String {
    format!("{} B", std::fs::metadata(dir.join(name)).unwrap().len())
}

#[test]
fn ascii_emits_nothing_but_sgr() {
    let dir = fixtures();
    let out = run(&["-f", "--protocol=ascii", "--color=always", "x.png"], &dir);
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(only_sgr(&text), "{text:?}");
    assert!(!text.contains('▀'));

    let out = run(&["-f", "--protocol=ascii", "--color=never", "x.png"], &dir);
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(!text.contains('\x1b'), "{text:?}");
    assert!(text.lines().last().unwrap().starts_with("x.png  40x20 PNG"));
}

#[test]
fn piped_output_is_the_file() {
    let dir = fixtures();
    let out = run(&["x.png"], &dir);
    assert_eq!(out.stdout, std::fs::read(dir.join("x.png")).unwrap());
    let out = run(&["shape.svg"], &dir);
    assert_eq!(out.stdout, std::fs::read(dir.join("shape.svg")).unwrap());
}

#[test]
fn kitty_and_sixel_can_be_forced() {
    let dir = fixtures();
    let out = run(&["-f", "--protocol=kitty", "-p", "x.png"], &dir);
    assert!(out.stdout.starts_with(b"\x1b_Ga=T,f=100,t=d,q=2,c="));
    assert!(out.stdout.ends_with(b"\x1b\\\n"));

    let out = run(&["-f", "--protocol=sixel", "-p", "x.png"], &dir);
    assert!(
        out.stdout.starts_with(b"\x1bP0;1;0q\"1;1;40;18"),
        "height rounded to sixel bands"
    );
    assert!(out.stdout.ends_with(b"\x1b\\\n"));
}

#[test]
fn svg_is_rendered() {
    let dir = fixtures();
    let out = run(
        &["-f", "--protocol=blocks", "--color=always", "shape.svg"],
        &dir,
    );
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.contains("38;2;0;255;0"), "{text:?}");
    assert!(text.contains("shape.svg  20x10 SVG"), "{text:?}");
}

#[test]
fn corrupt_images_are_reported_and_skipped() {
    let dir = fixtures();
    let out = run(
        &[
            "-f",
            "--protocol=ascii",
            "--color=never",
            "broken.png",
            "notes.txt",
        ],
        &dir,
    );
    assert_eq!(out.status.code(), Some(1));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.starts_with("scat: broken.png: cannot decode image: "),
        "{err}"
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains("hello"));
}

#[test]
fn image_renderer_rule_and_raw_rule() {
    let dir = fixtures();
    let config = dir.join("raw-images.toml");
    std::fs::write(
        &config,
        "[[rule]]\nmatch = { mime = \"image/*\" }\nrenderer = \"raw\"\n",
    )
    .unwrap();
    let out = run(&["-f", "--config", config.to_str().unwrap(), "x.png"], &dir);
    assert_eq!(out.stdout, std::fs::read(dir.join("x.png")).unwrap());
}
