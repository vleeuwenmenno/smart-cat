//! Transparent decompression through the binary: every format, inner-type detection,
//! tarballs, the bomb cap and untouched piped bytes.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Output};

const SCAT: &str = env!("CARGO_BIN_EXE_scat");

const LOG: &str = "2026-10-23 13:42:10 INFO start\n2026-10-23 13:42:11 ERROR boom\n";
const NOTES: &str = "plain notes\nsecond line\n";

fn gzip(data: &[u8]) -> Vec<u8> {
    let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    enc.write_all(data).unwrap();
    enc.finish().unwrap()
}

fn bzip2(data: &[u8]) -> Vec<u8> {
    let mut enc = bzip2::write::BzEncoder::new(Vec::new(), bzip2::Compression::best());
    enc.write_all(data).unwrap();
    enc.finish().unwrap()
}

fn xz(data: &[u8]) -> Vec<u8> {
    let mut enc =
        lzma_rust2::XzWriter::new(Vec::new(), lzma_rust2::XzOptions::with_preset(1)).unwrap();
    enc.write_all(data).unwrap();
    enc.finish().unwrap()
}

fn zstd(data: &[u8]) -> Vec<u8> {
    ruzstd::encoding::compress_to_vec(data, ruzstd::encoding::CompressionLevel::Fastest)
}

fn dir(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("compressed")
        .join(name);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn scat(dir: &std::path::Path, args: &[&str]) -> Output {
    Command::new(SCAT)
        .args(args)
        .current_dir(dir)
        .env_remove("SCAT_CONFIG")
        .env_remove("SCAT_COLOR")
        .env_remove("XDG_CONFIG_HOME")
        .env("HOME", dir)
        .env("LC_ALL", "C.UTF-8")
        .env("COLUMNS", "60")
        .output()
        .unwrap()
}

fn text(out: &Output) -> String {
    String::from_utf8(out.stdout.clone()).unwrap()
}

#[test]
fn every_format_is_decompressed() {
    let d = dir("formats");
    let cases: [(&str, Vec<u8>); 4] = [
        ("notes.txt.gz", gzip(NOTES.as_bytes())),
        ("notes.txt.bz2", bzip2(NOTES.as_bytes())),
        ("notes.txt.xz", xz(NOTES.as_bytes())),
        ("notes.txt.zst", zstd(NOTES.as_bytes())),
    ];
    for (name, bytes) in cases {
        std::fs::write(d.join(name), bytes).unwrap();
        let out = scat(&d, &["-f", "--color=never", name]);
        assert!(out.status.success(), "{name}: {:?}", out.stderr);
        assert_eq!(
            text(&out),
            "   1 │ plain notes\n   2 │ second line\n",
            "{name}"
        );
    }
}

#[test]
fn detection_uses_magic_not_the_extension() {
    let d = dir("magic");
    std::fs::write(d.join("blob"), gzip(NOTES.as_bytes())).unwrap();
    let out = scat(&d, &["-f", "--color=never", "-p", "blob"]);
    assert_eq!(text(&out), NOTES);
}

#[test]
fn multi_member_gzip_is_read_to_the_end() {
    let d = dir("multi");
    let mut bytes = gzip(b"one\n");
    bytes.extend(gzip(b"two\n"));
    std::fs::write(d.join("m.txt.gz"), bytes).unwrap();
    let out = scat(&d, &["-f", "--color=never", "-p", "m.txt.gz"]);
    assert_eq!(text(&out), "one\ntwo\n");
}

#[test]
fn inner_name_picks_the_renderer() {
    let d = dir("inner");
    std::fs::write(d.join("x.md.gz"), gzip(b"# Title\n\nsome *text*\n")).unwrap();
    let out = scat(&d, &["-f", "--color=never", "x.md.gz"]);
    let got = text(&out);
    assert!(got.starts_with("Title\n━━━"), "{got:?}");
    assert!(got.ends_with("\nsome text\n"), "{got:?}");

    std::fs::write(d.join("x.log.gz"), gzip(LOG.as_bytes())).unwrap();
    let out = scat(&d, &["-f", "--color=always", "-p", "x.log.gz"]);
    let got = text(&out);
    assert!(got.contains("\x1b[2m2026-10-23 13:42:10\x1b[0m"), "{got:?}");
    assert!(got.contains("\x1b[31mERROR\x1b[0m"), "{got:?}");
}

#[test]
fn tarballs_are_not_shown_as_text() {
    let d = dir("tar");
    let mut tar = vec![0u8; 1024];
    tar[..5].copy_from_slice(b"a.txt");
    tar[257..262].copy_from_slice(b"ustar");
    tar[512..524].copy_from_slice(b"hello world\n");
    // By name and by content (a tarball with an unhelpful name).
    for name in ["a.tar.gz", "a.tgz", "unhelpful.gz"] {
        std::fs::write(d.join(name), gzip(&tar)).unwrap();
        let out = scat(&d, &["-f", "--color=never", name]);
        assert!(out.stdout.is_empty(), "{name}: {:?}", text(&out));
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(err.contains("binary file"), "{name}: {err}");
    }
}

#[test]
fn bomb_is_cut_off_with_a_note() {
    let d = dir("bomb");
    std::fs::write(d.join("big.txt.gz"), gzip(&b"0123456789\n".repeat(10_000))).unwrap();
    std::fs::write(
        d.join("config.toml"),
        "[defaults]\nmax_decompressed_bytes = 1000\n",
    )
    .unwrap();
    let out = scat(
        &d,
        &[
            "-f",
            "--color=never",
            "-p",
            "--config",
            "config.toml",
            "big.txt.gz",
        ],
    );
    assert!(out.stdout.len() <= 1000, "{}", out.stdout.len());
    assert!(!out.stdout.is_empty());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("decompressed output stopped"), "{err}");
}

#[test]
fn output_exactly_at_the_cap_is_not_truncated() {
    let d = dir("exact");
    let mut data = vec![b'a'; 99];
    data.push(b'\n');
    std::fs::write(d.join("e.txt.gz"), gzip(&data)).unwrap();
    std::fs::write(
        d.join("config.toml"),
        "[defaults]\nmax_decompressed_bytes = 100\n",
    )
    .unwrap();
    let out = scat(
        &d,
        &[
            "-f",
            "--color=never",
            "-p",
            "--config",
            "config.toml",
            "e.txt.gz",
        ],
    );
    assert_eq!(out.stdout.len(), 100);
    assert!(out.stderr.is_empty(), "{:?}", out.stderr);
}

#[test]
fn piped_bytes_stay_compressed() {
    let d = dir("piped");
    let bytes = gzip(NOTES.as_bytes());
    std::fs::write(d.join("notes.txt.gz"), &bytes).unwrap();
    let out = scat(&d, &["notes.txt.gz"]);
    assert_eq!(out.stdout, bytes);
}

#[test]
fn raw_rule_keeps_the_bytes() {
    let d = dir("raw");
    let bytes = gzip(NOTES.as_bytes());
    std::fs::write(d.join("notes.txt.gz"), &bytes).unwrap();
    std::fs::write(
        d.join("config.toml"),
        "[[rule]]\nmatch = { ext = \"gz\" }\nrenderer = \"raw\"\n",
    )
    .unwrap();
    let out = scat(&d, &["-f", "--config", "config.toml", "notes.txt.gz"]);
    assert_eq!(out.stdout, bytes);
}

#[test]
fn corrupt_stream_is_an_error() {
    let d = dir("corrupt");
    let mut bytes = gzip(&b"line\n".repeat(5000));
    bytes.truncate(bytes.len() / 2);
    std::fs::write(d.join("c.txt.gz"), bytes).unwrap();
    let out = scat(&d, &["-f", "--color=never", "c.txt.gz"]);
    assert!(!out.status.success());
}
