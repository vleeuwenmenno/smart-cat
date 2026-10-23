//! Archive listing through the binary: zip, tar, tarballs, 7z and deb fixtures built with
//! the crates' own writers, exact tree output, caps, hostile names and piped bytes.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const SCAT: &str = env!("CARGO_BIN_EXE_scat");

/// 2023-11-14 22:13:20 UTC.
const MTIME: u64 = 1_700_000_000;

fn dir(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("archive")
        .join(name);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn scat(dir: &Path, args: &[&str]) -> Output {
    Command::new(SCAT)
        .args(args)
        .current_dir(dir)
        .env_remove("SCAT_CONFIG")
        .env_remove("SCAT_COLOR")
        .env_remove("LS_COLORS")
        .env_remove("XDG_CONFIG_HOME")
        .env("HOME", dir)
        .env("LC_ALL", "C.UTF-8")
        .env("COLUMNS", "80")
        .output()
        .unwrap()
}

fn text(out: &Output) -> String {
    String::from_utf8(out.stdout.clone()).unwrap()
}

fn tar_header(path: &[u8], kind: tar::EntryType, size: u64, mode: u32) -> tar::Header {
    let mut header = tar::Header::new_gnu();
    // Set the name bytes directly: hostile names must get through the writer.
    header.as_old_mut().name[..path.len()].copy_from_slice(path);
    header.set_entry_type(kind);
    header.set_size(size);
    header.set_mode(mode);
    header.set_mtime(MTIME);
    header.set_cksum();
    header
}

/// A tar with nested directories, an executable and a symlink.
fn sample_tar() -> Vec<u8> {
    let mut builder = tar::Builder::new(Vec::new());
    let entries: [(&str, tar::EntryType, &[u8], u32); 6] = [
        ("proj/", tar::EntryType::Directory, b"", 0o755),
        ("proj/src/", tar::EntryType::Directory, b"", 0o755),
        (
            "proj/src/main.rs",
            tar::EntryType::Regular,
            b"fn main() {}\n",
            0o644,
        ),
        (
            "proj/run.sh",
            tar::EntryType::Regular,
            b"#!/bin/sh\n",
            0o755,
        ),
        ("proj/README", tar::EntryType::Regular, b"hello\n", 0o644),
        ("proj/latest", tar::EntryType::Symlink, b"", 0o777),
    ];
    for (path, kind, data, mode) in entries {
        let mut header = tar_header(path.as_bytes(), kind, data.len() as u64, mode);
        if kind == tar::EntryType::Symlink {
            header.set_link_name("README").unwrap();
            header.set_cksum();
        }
        builder.append(&header, data).unwrap();
    }
    builder.into_inner().unwrap()
}

const SAMPLE_TREE: &str = "\
└── proj/                       2023-11-14
    ├── src/                    2023-11-14
    │   └── main.rs       13 B  2023-11-14
    ├── latest -> README        2023-11-14
    ├── README             6 B  2023-11-14
    └── run.sh*           10 B  2023-11-14
";

fn gzip(data: &[u8]) -> Vec<u8> {
    let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    enc.write_all(data).unwrap();
    enc.finish().unwrap()
}

fn zip_options() -> zip::write::SimpleFileOptions {
    zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Stored)
        .last_modified_time(zip::DateTime::from_date_and_time(2023, 11, 14, 22, 13, 20).unwrap())
}

fn sample_zip() -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = zip_options();
    zip.add_directory("proj/", options).unwrap();
    zip.add_directory("proj/src/", options).unwrap();
    zip.start_file("proj/src/main.rs", options).unwrap();
    zip.write_all(b"fn main() {}\n").unwrap();
    zip.start_file("proj/run.sh", options.unix_permissions(0o755))
        .unwrap();
    zip.write_all(b"#!/bin/sh\n").unwrap();
    zip.start_file("proj/README", options).unwrap();
    zip.write_all(b"hello\n").unwrap();
    zip.add_symlink("proj/latest", "README", options).unwrap();
    zip.finish().unwrap().into_inner()
}

fn tree_of(got: &str) -> String {
    // The header table ends at the first blank line; the rest is the tree.
    got.split_once("\n\n")
        .expect("blank line after header")
        .1
        .to_string()
}

#[test]
fn zip_lists_a_tree() {
    let d = dir("zip");
    std::fs::write(d.join("sample.zip"), sample_zip()).unwrap();
    let out = scat(&d, &["-f", "--color=never", "sample.zip"]);
    assert!(out.status.success(), "{:?}", out.stderr);
    let got = text(&out);
    assert!(
        got.starts_with("sample.zip\n  Type        zip archive\n  Entries     6\n"),
        "{got}"
    );
    assert!(got.contains("  Size        29 B uncompressed\n"), "{got}");
    assert!(got.contains("  Compressed  "), "{got}");
    assert_eq!(tree_of(&got), SAMPLE_TREE);
}

#[test]
fn tar_lists_the_same_tree() {
    let d = dir("tar");
    std::fs::write(d.join("sample.tar"), sample_tar()).unwrap();
    let out = scat(&d, &["-f", "--color=never", "sample.tar"]);
    assert!(out.status.success(), "{:?}", out.stderr);
    let got = text(&out);
    assert!(
        got.starts_with("sample.tar\n  Type     tar archive\n  Entries  6\n"),
        "{got}"
    );
    assert!(!got.contains("Compressed"), "{got}");
    let (head, tree) = got.split_once("\n\n").unwrap();
    assert!(head.contains("Size     29 B uncompressed"), "{head}");
    assert_eq!(tree, SAMPLE_TREE);
}

#[test]
fn compressed_tarballs_are_listed_not_hex_dumped() {
    let d = dir("tarballs");
    let tar = sample_tar();
    for name in ["a.tar.gz", "a.tgz", "unhelpful.gz"] {
        std::fs::write(d.join(name), gzip(&tar)).unwrap();
        let out = scat(&d, &["-f", "--color=never", name]);
        assert!(out.status.success(), "{name}: {:?}", out.stderr);
        let got = text(&out);
        assert!(
            got.contains("Type        tar archive (gzip)"),
            "{name}: {got:?}"
        );
        assert!(got.contains("└── proj/"), "{name}: {got:?}");
        assert!(got.contains("run.sh*"), "{name}: {got:?}");
        assert!(!got.starts_with("00000000"), "{name}: {got:?}");
        assert!(got.contains("  Compressed  "), "{name}: {got:?}");
    }
}

#[test]
fn every_compression_is_listed() {
    let d = dir("compressions");
    let tar = sample_tar();
    let mut xz =
        lzma_rust2::XzWriter::new(Vec::new(), lzma_rust2::XzOptions::with_preset(1)).unwrap();
    xz.write_all(&tar).unwrap();
    let mut bz = bzip2::write::BzEncoder::new(Vec::new(), bzip2::Compression::best());
    bz.write_all(&tar).unwrap();
    let zst =
        ruzstd::encoding::compress_to_vec(&tar[..], ruzstd::encoding::CompressionLevel::Fastest);
    let cases = [
        ("a.tar.xz", xz.finish().unwrap(), "xz"),
        ("a.tar.bz2", bz.finish().unwrap(), "bzip2"),
        ("a.tar.zst", zst, "zstd"),
    ];
    for (name, bytes, label) in cases {
        std::fs::write(d.join(name), bytes).unwrap();
        let out = scat(&d, &["-f", "--color=never", name]);
        assert!(out.status.success(), "{name}: {:?}", out.stderr);
        let got = text(&out);
        assert!(
            got.contains(&format!("tar archive ({label})")),
            "{name}: {got:?}"
        );
        assert!(got.contains("main.rs"), "{name}: {got:?}");
    }
}

#[test]
fn ascii_tree_without_utf8() {
    let d = dir("ascii");
    std::fs::write(d.join("sample.tar"), sample_tar()).unwrap();
    let out = Command::new(SCAT)
        .args(["-f", "--color=never", "sample.tar"])
        .current_dir(&d)
        .env_remove("SCAT_CONFIG")
        .env("HOME", &d)
        .env("LC_ALL", "C")
        .env_remove("LANG")
        .env("COLUMNS", "80")
        .output()
        .unwrap();
    let got = text(&out);
    let ascii = SAMPLE_TREE
        .replace("└── ", "`-- ")
        .replace("├── ", "|-- ")
        .replace("│   ", "|   ");
    assert_eq!(tree_of(&got), ascii);
}

#[test]
fn max_entries_adds_a_footer() {
    let d = dir("cap");
    let mut builder = tar::Builder::new(Vec::new());
    for i in 0..30 {
        let header = tar_header(
            format!("f{i:02}.txt").as_bytes(),
            tar::EntryType::Regular,
            1,
            0o644,
        );
        builder.append(&header, &b"x"[..]).unwrap();
    }
    std::fs::write(d.join("many.tar"), builder.into_inner().unwrap()).unwrap();
    std::fs::write(d.join("config.toml"), "[archive]\nmax_entries = 10\n").unwrap();
    let out = scat(
        &d,
        &["-f", "--color=never", "--config", "config.toml", "many.tar"],
    );
    let got = text(&out);
    assert!(got.contains("  Entries  30\n"), "{got}");
    let tree = tree_of(&got);
    assert_eq!(tree.lines().count(), 11, "{tree}");
    assert!(
        tree.ends_with("└── f09.txt  1 B  2023-11-14\n… 20 more entries\n"),
        "{tree}"
    );
    // Default cap is 500: all 30 show.
    let all = text(&scat(&d, &["-f", "--color=never", "many.tar"]));
    assert!(!all.contains("more entries"), "{all}");
}

#[test]
fn hostile_names_are_sanitized() {
    let d = dir("hostile");
    let mut builder = tar::Builder::new(Vec::new());
    for name in [
        &b"evil\x1b[31m\nname"[..],
        b"../../etc/passwd",
        b"/abs/path",
    ] {
        let header = tar_header(name, tar::EntryType::Regular, 0, 0o644);
        builder.append(&header, &b""[..]).unwrap();
    }
    let header = tar_header(b"link", tar::EntryType::Symlink, 0, 0o777);
    let mut header = header;
    header.set_link_name("\x1b]0;pwn\x07").unwrap();
    header.set_cksum();
    builder.append(&header, &b""[..]).unwrap();
    std::fs::write(d.join("evil.tar"), builder.into_inner().unwrap()).unwrap();

    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    zip.start_file("a\x1b[2Jb\nc.txt", zip_options()).unwrap();
    std::fs::write(d.join("evil.zip"), zip.finish().unwrap().into_inner()).unwrap();

    for name in ["evil.tar", "evil.zip"] {
        let out = scat(&d, &["-f", "--color=never", name]);
        assert!(out.status.success(), "{name}: {:?}", out.stderr);
        let got = text(&out);
        assert!(!got.contains('\x1b'), "{name}: {got:?}");
        assert!(!got.contains('\x07'), "{name}: {got:?}");
        // One tree row per entry: the newline inside a name does not split the row.
        let tree = tree_of(&got);
        assert_eq!(
            tree.lines().count(),
            if name == "evil.tar" { 9 } else { 1 },
            "{name}: {tree:?}"
        );
        if name == "evil.tar" {
            assert!(tree.contains("^[[31m^Jname"), "{tree:?}");
            // Traversal and absolute paths stay visible as they are stored.
            assert!(tree.contains("../"), "{tree:?}");
            assert!(tree.contains("── /\n"), "{tree:?}");
        }
    }
}

#[test]
fn sevenz_and_deb_are_listed() {
    let d = dir("others");
    let mut sz = sevenz_rust2::ArchiveWriter::create(d.join("a.7z")).unwrap();
    sz.push_archive_entry(
        sevenz_rust2::ArchiveEntry::new_file("docs/readme.txt"),
        Some(&b"hello\n"[..]),
    )
    .unwrap();
    sz.finish().unwrap();
    let out = scat(&d, &["-f", "--color=never", "a.7z"]);
    assert!(
        out.status.success(),
        "{:?}",
        String::from_utf8_lossy(&out.stderr)
    );
    let got = text(&out);
    assert!(got.contains("Type        7z archive"), "{got}");
    assert!(got.contains("└── docs/\n    └── readme.txt  6 B"), "{got}");

    // A .deb is an ar archive holding control.tar.gz and data.tar.gz.
    let mut control = tar::Builder::new(Vec::new());
    control
        .append(
            &tar_header(b"control", tar::EntryType::Regular, 4, 0o644),
            &b"abc\n"[..],
        )
        .unwrap();
    let mut data = tar::Builder::new(Vec::new());
    data.append(
        &tar_header(b"./usr/bin/tool", tar::EntryType::Regular, 2, 0o755),
        &b"x\n"[..],
    )
    .unwrap();
    let mut deb = b"!<arch>\n".to_vec();
    for (name, bytes) in [
        ("debian-binary", b"2.0\n".to_vec()),
        ("control.tar.gz", gzip(&control.into_inner().unwrap())),
        ("data.tar.gz", gzip(&data.into_inner().unwrap())),
    ] {
        deb.extend_from_slice(
            format!(
                "{name:<16}{:<12}{:<6}{:<6}{:<8}{:<10}`\n",
                0,
                0,
                0,
                "100644",
                bytes.len()
            )
            .as_bytes(),
        );
        deb.extend_from_slice(&bytes);
        if bytes.len() % 2 == 1 {
            deb.push(b'\n');
        }
    }
    std::fs::write(d.join("tool.deb"), deb).unwrap();
    let out = scat(&d, &["-f", "--color=never", "tool.deb"]);
    assert!(
        out.status.success(),
        "{:?}",
        String::from_utf8_lossy(&out.stderr)
    );
    let got = text(&out);
    assert!(got.contains("Type        Debian package"), "{got}");
    assert!(
        got.contains("control.tar.gz (1 entry)\n└── control  4 B"),
        "{got}"
    );
    assert!(
        got.contains("data.tar.gz (1 entry)\n└── usr/\n    └── bin/\n        └── tool*  2 B"),
        "{got}"
    );
}

#[test]
fn piped_output_is_the_archive_bytes() {
    let d = dir("piped");
    let zip = sample_zip();
    let tar = sample_tar();
    let tgz = gzip(&tar);
    for (name, bytes) in [("a.zip", zip), ("a.tar", tar), ("a.tar.gz", tgz)] {
        std::fs::write(d.join(name), &bytes).unwrap();
        let out = scat(&d, &[name]);
        assert_eq!(out.stdout, bytes, "{name}");
    }
}

#[test]
fn corrupt_archives_are_errors_and_scat_continues() {
    let d = dir("corrupt");
    std::fs::write(d.join("bad.zip"), b"PK\x03\x04 this is not a zip").unwrap();
    let mut bad_gz = vec![0x1f, 0x8b, 0x08, 0, 0, 0, 0, 0, 0, 3];
    bad_gz.extend_from_slice(&[0xff; 64]);
    std::fs::write(d.join("bad.tar.gz"), bad_gz).unwrap();
    std::fs::write(
        d.join("bad.7z"),
        b"7z\xbc\xaf\x27\x1c\0\x04garbage garbage garbage",
    )
    .unwrap();
    std::fs::write(d.join("after.txt"), "still here\n").unwrap();
    for name in ["bad.zip", "bad.tar.gz", "bad.7z"] {
        let out = scat(&d, &["-f", "--color=never", "-p", name, "after.txt"]);
        assert_eq!(out.status.code(), Some(1), "{name}");
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(err.starts_with(&format!("scat: {name}: ")), "{name}: {err}");
        assert!(text(&out).contains("still here"), "{name}: {}", text(&out));
    }
}

#[test]
fn sevenz_header_sizes_are_checked_before_parsing() {
    // A 32-byte 7z file whose end header claims 2^63 bytes: sevenz-rust2 0.20 panics
    // with a capacity overflow allocating it, which aborts a release build.
    let d = dir("sevenz-huge-header");
    let mut start = Vec::new();
    start.extend_from_slice(&0u64.to_le_bytes());
    start.extend_from_slice(&(1u64 << 63).to_le_bytes());
    start.extend_from_slice(&0u32.to_le_bytes());
    let mut file = b"7z\xbc\xaf\x27\x1c\0\x04".to_vec();
    file.extend_from_slice(&crc32(&start).to_le_bytes());
    file.extend_from_slice(&start);
    assert_eq!(file.len(), 32);
    std::fs::write(d.join("huge.7z"), &file).unwrap();
    std::fs::write(d.join("after.txt"), "still here\n").unwrap();
    let out = scat(&d, &["-f", "--color=never", "huge.7z", "after.txt"]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(
        String::from_utf8_lossy(&out.stderr),
        "scat: huge.7z: not a valid 7z archive: the header lies outside the file\n"
    );
    assert!(text(&out).contains("still here"));
}

#[test]
fn sevenz_from_a_pipe_is_listed_through_the_helper() {
    let d = dir("sevenz-pipe");
    let mut sz = sevenz_rust2::ArchiveWriter::create(d.join("p.7z")).unwrap();
    sz.push_archive_entry(
        sevenz_rust2::ArchiveEntry::new_file("x/y.txt"),
        Some(&b"abc"[..]),
    )
    .unwrap();
    sz.finish().unwrap();
    let out = Command::new(SCAT)
        .args(["-f", "--color=never", "-"])
        .current_dir(&d)
        .env("HOME", &d)
        .env("LC_ALL", "C.UTF-8")
        .env("COLUMNS", "80")
        .stdin(std::fs::File::open(d.join("p.7z")).unwrap())
        .output()
        .unwrap();
    let got = text(&out);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(got.contains("└── x/\n    └── y.txt  3 B"), "{got}");
}

/// CRC-32 (IEEE), for the 7z start header.
fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &b in data {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xedb8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

#[test]
fn raw_and_hex_keep_the_bytes() {
    let d = dir("renderers");
    std::fs::write(d.join("a.tar.gz"), gzip(&sample_tar())).unwrap();
    let raw = scat(&d, &["--raw", "a.tar.gz"]);
    assert_eq!(raw.stdout, gzip(&sample_tar()));
    std::fs::write(
        d.join("hex.toml"),
        "[[rule]]\nmatch = { ext = \"gz\" }\nrenderer = \"hex\"\n",
    )
    .unwrap();
    let hex = text(&scat(
        &d,
        &["-f", "--config", "hex.toml", "--color=never", "a.tar.gz"],
    ));
    assert!(hex.starts_with("00000000  1f 8b"), "{hex:?}");
}
