//! Font view through the binary: a minimal TrueType font built by the test (and its
//! WOFF wrapping), plus system fonts when they are installed.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const SCAT: &str = env!("CARGO_BIN_EXE_scat");

/// A fresh empty directory for one test.
fn dir(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("fonts")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn scat(args: &[&str], files: &[&Path]) -> Output {
    Command::new(SCAT)
        .args(args)
        .args(files)
        .env("COLUMNS", "100")
        .env("LINES", "60")
        .env("LANG", "C.UTF-8")
        .env("LC_ALL", "C.UTF-8")
        .env("SCAT_CONFIG", "/nonexistent")
        .output()
        .unwrap()
}

fn text(out: &Output) -> String {
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout.clone()).unwrap()
}

// --- A minimal TrueType font -------------------------------------------------------

fn be16(v: &mut Vec<u8>, x: u16) {
    v.extend_from_slice(&x.to_be_bytes());
}

fn be32(v: &mut Vec<u8>, x: u32) {
    v.extend_from_slice(&x.to_be_bytes());
}

const LICENSE: &str = "Test license \x1b[31mred\x1b]0;title\x07\nsecond line";

/// Two glyphs (`.notdef` and a box for every ASCII letter), bold weight, monospaced,
/// with the usual name entries and a hostile license string.
fn tiny_font() -> Vec<u8> {
    let mut head = Vec::new();
    be32(&mut head, 0x0001_0000);
    be32(&mut head, 0x0001_0000);
    be32(&mut head, 0);
    be32(&mut head, 0x5F0F_3CF5);
    be16(&mut head, 0x000B);
    be16(&mut head, 1000);
    head.extend_from_slice(&[0; 16]);
    for v in [0u16, 0, 600, 700, 0, 8, 2, 0, 0] {
        be16(&mut head, v);
    }

    let mut hhea = Vec::new();
    be32(&mut hhea, 0x0001_0000);
    for v in [
        800u16,
        (-200i16) as u16,
        0,
        600,
        0,
        0,
        600,
        1,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        2,
    ] {
        be16(&mut hhea, v);
    }

    let mut maxp = Vec::new();
    be32(&mut maxp, 0x0001_0000);
    for v in [2u16, 4, 1, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0] {
        be16(&mut maxp, v);
    }

    let mut os2 = Vec::new();
    for v in [4u16, 600, 700, 5, 0] {
        be16(&mut os2, v);
    }
    os2.extend_from_slice(&[0; 20 + 2 + 10 + 16]);
    os2.extend_from_slice(b"SCAT");
    for v in [0x0020u16, 0x41, 0x7a, 800, (-200i16) as u16, 0, 800, 200] {
        be16(&mut os2, v);
    }
    os2.extend_from_slice(&[0; 8]);
    for v in [500u16, 700, 0, 32, 0] {
        be16(&mut os2, v);
    }
    assert_eq!(os2.len(), 96);

    let mut post = Vec::new();
    be32(&mut post, 0x0003_0000);
    be32(&mut post, 0);
    be16(&mut post, (-100i16) as u16);
    be16(&mut post, 50);
    be32(&mut post, 1); // isFixedPitch
    post.extend_from_slice(&[0; 16]);

    // cmap format 4: A-Z and a-z to glyph 1.
    let mut sub = Vec::new();
    for v in [
        4u16, 40, 0, 6, 4, 1, 2, 0x5a, 0x7a, 0xffff, 0, 0x41, 0x61, 0xffff,
    ] {
        be16(&mut sub, v);
    }
    for d in [1i16 - 0x41, 1 - 0x61, 1] {
        be16(&mut sub, d as u16);
    }
    sub.extend_from_slice(&[0; 6]);
    let mut cmap = Vec::new();
    for v in [0u16, 1, 3, 1] {
        be16(&mut cmap, v);
    }
    be32(&mut cmap, 12);
    cmap.extend_from_slice(&sub);

    // One box glyph, padded to 36 bytes.
    let mut glyf = Vec::new();
    for v in [1u16, 50, 0, 550, 700, 3, 0] {
        be16(&mut glyf, v);
    }
    glyf.extend_from_slice(&[1, 1, 1, 1]);
    for v in [50i16, 0, 500, 0, 0, 700, 0, -700] {
        be16(&mut glyf, v as u16);
    }
    glyf.extend_from_slice(&[0, 0]);
    let mut loca = Vec::new();
    for v in [0u16, 0, 18] {
        be16(&mut loca, v);
    }
    let mut hmtx = Vec::new();
    for v in [600u16, 0, 600, 50] {
        be16(&mut hmtx, v);
    }

    let names: [(u16, &str); 8] = [
        (1, "Scat Test"),
        (2, "Bold"),
        (4, "Scat Test Bold"),
        (5, "Version 1.000"),
        (6, "ScatTest-Bold"),
        (9, "Test Designer"),
        (13, LICENSE),
        (14, "https://example.org/license"),
    ];
    let mut name = Vec::new();
    let mut strings = Vec::new();
    be16(&mut name, 0);
    be16(&mut name, names.len() as u16);
    be16(&mut name, 6 + 12 * names.len() as u16);
    for (id, s) in names {
        let utf16: Vec<u8> = s.encode_utf16().flat_map(u16::to_be_bytes).collect();
        for v in [3u16, 1, 0x409, id, utf16.len() as u16, strings.len() as u16] {
            be16(&mut name, v);
        }
        strings.extend_from_slice(&utf16);
    }
    name.extend_from_slice(&strings);

    sfnt(&mut [
        (*b"OS/2", os2),
        (*b"cmap", cmap),
        (*b"glyf", glyf),
        (*b"head", head),
        (*b"hhea", hhea),
        (*b"hmtx", hmtx),
        (*b"loca", loca),
        (*b"maxp", maxp),
        (*b"name", name),
        (*b"post", post),
    ])
}

fn checksum(data: &[u8]) -> u32 {
    data.chunks(4).fold(0u32, |sum, c| {
        let mut b = [0u8; 4];
        b[..c.len()].copy_from_slice(c);
        sum.wrapping_add(u32::from_be_bytes(b))
    })
}

fn sfnt(tables: &mut [([u8; 4], Vec<u8>)]) -> Vec<u8> {
    tables.sort_by_key(|t| t.0);
    let n = tables.len() as u16;
    let mut out = Vec::new();
    be32(&mut out, 0x0001_0000);
    for v in [n, 128, 3, n * 16 - 128] {
        be16(&mut out, v);
    }
    let mut offset = 12 + 16 * tables.len();
    for (tag, data) in tables.iter() {
        out.extend_from_slice(tag);
        be32(&mut out, checksum(data));
        be32(&mut out, offset as u32);
        be32(&mut out, data.len() as u32);
        offset += data.len().div_ceil(4) * 4;
    }
    for (_, data) in tables.iter() {
        out.extend_from_slice(data);
        out.resize(out.len().div_ceil(4) * 4, 0);
    }
    out
}

/// Wraps an sfnt in WOFF 1.0, zlib-compressing the tables that get smaller.
fn woff(font: &[u8]) -> Vec<u8> {
    let u16_at = |i: usize| u16::from_be_bytes([font[i], font[i + 1]]);
    let u32_at = |i: usize| u32::from_be_bytes(font[i..i + 4].try_into().unwrap());
    let n = usize::from(u16_at(4));
    let mut dir = Vec::new();
    let mut data = Vec::new();
    let start = 44 + 20 * n;
    for i in 0..n {
        let rec = 12 + 16 * i;
        let (off, len) = (u32_at(rec + 8) as usize, u32_at(rec + 12) as usize);
        let table = &font[off..off + len];
        let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best());
        z.write_all(table).unwrap();
        let z = z.finish().unwrap();
        let stored = if z.len() < len { z } else { table.to_vec() };
        dir.extend_from_slice(&font[rec..rec + 4]);
        be32(&mut dir, (start + data.len()) as u32);
        be32(&mut dir, stored.len() as u32);
        be32(&mut dir, len as u32);
        be32(&mut dir, u32_at(rec + 4));
        data.extend_from_slice(&stored);
        data.resize(data.len().div_ceil(4) * 4, 0);
    }
    let mut out = b"wOFF".to_vec();
    be32(&mut out, 0x0001_0000);
    be32(&mut out, (start + data.len()) as u32);
    be16(&mut out, n as u16);
    be16(&mut out, 0);
    be32(&mut out, font.len() as u32);
    out.extend_from_slice(&[0; 24]);
    out.extend_from_slice(&dir);
    out.extend_from_slice(&data);
    out
}

/// The first installed font that fontconfig lists for `pattern`, if any.
fn system_font(pattern: &str, filter: impl Fn(&str) -> bool) -> Option<PathBuf> {
    let out = Command::new("fc-list")
        .args([pattern, "file"])
        .output()
        .ok()?;
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|l| l.trim().trim_end_matches(':').to_string())
        .filter(|p| filter(p))
        .map(PathBuf::from)
        .find(|p| p.is_file())
}

/// Runs of spaces as one, so checks do not depend on the key column width.
fn squeeze(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if !(c == ' ' && out.ends_with(' ')) {
            out.push(c);
        }
    }
    out
}

// --- Tests ---------------------------------------------------------------------------

#[test]
fn info_table_of_the_test_font() {
    let d = dir("info");
    let path = d.join("ScatTest-Bold.ttf");
    std::fs::write(&path, tiny_font()).unwrap();
    let out = squeeze(&text(&scat(&["-f", "--color=never"], &[&path])));
    for want in [
        "format OpenType",
        "family Scat Test",
        "style Bold",
        "full name Scat Test Bold",
        "PostScript ScatTest-Bold",
        "version Version 1.000",
        "designer Test Designer",
        "weight 700 (Bold)",
        "width 5 (Normal)",
        "italic no",
        "monospace yes",
        "outlines TrueType",
        "glyphs 2",
        "units/em 1000",
        "characters 52",
        "coverage Latin 52",
        "license URL https://example.org/license",
        "file size 904 B",
    ] {
        assert!(out.contains(want), "missing {want:?} in\n{out}");
    }
    // The license keeps its first line, with the escapes shown, not passed on.
    assert!(!out.contains('\x1b'), "{out:?}");
    assert!(!out.contains('\x07'), "{out:?}");
    assert!(
        out.contains("license Test license ^[[31mred^[]0;title^G\n"),
        "{out}"
    );
    assert!(!out.contains("second line"), "{out}");
    // Without --protocol, -f into a pipe gives the table only.
    assert!(!out.contains('▀'), "{out}");
}

#[test]
fn specimen_is_drawn_with_blocks() {
    let d = dir("blocks");
    let path = d.join("ScatTest-Bold.ttf");
    std::fs::write(&path, tiny_font()).unwrap();
    let args = ["-f", "--protocol=blocks", "--color=always"];
    let out = text(&scat(&args, &[&path]));
    let (table, image) = out.split_once("\n\n").expect("table, blank line, specimen");
    assert!(table.contains("Scat Test"), "{table}");
    assert!(image.contains('▀'), "{image}");
    // Dark boxes on light paper: some cells mix a dark and a light colour.
    assert!(image.lines().count() >= 5, "{image}");
}

#[test]
fn ascii_protocol_gives_the_table_only() {
    let d = dir("ascii");
    let path = d.join("ScatTest-Bold.ttf");
    std::fs::write(&path, tiny_font()).unwrap();
    let out = squeeze(&text(&scat(&["-f", "--protocol=ascii"], &[&path])));
    assert!(out.contains("Scat Test"), "{out}");
    assert!(out.trim_end().ends_with("file size 904 B"), "{out}");
}

#[test]
fn woff_shows_the_same_table() {
    let d = dir("woff");
    let font = tiny_font();
    let ttf = d.join("t.ttf");
    let woff_path = d.join("t.woff");
    std::fs::write(&ttf, &font).unwrap();
    std::fs::write(&woff_path, woff(&font)).unwrap();
    let a = text(&scat(&["-f", "--color=never"], &[&ttf]));
    let b = text(&scat(&["-f", "--color=never"], &[&woff_path]));
    assert!(squeeze(&b).contains("format WOFF\n"), "{b}");
    let strip = |s: &str| {
        s.lines()
            .filter(|l| {
                !l.contains("format")
                    && !l.contains("file size")
                    && !l.ends_with(".ttf")
                    && !l.ends_with(".woff")
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    assert_eq!(strip(&a), strip(&b));
}

#[test]
fn woff2_from_the_system_tool_matches_the_font() {
    let Some(tool) = which("woff2_compress") else {
        eprintln!("woff2_compress not installed; skipped");
        return;
    };
    let d = dir("woff2");
    let ttf = d.join("t.ttf");
    std::fs::write(&ttf, tiny_font()).unwrap();
    let status = Command::new(tool).arg(&ttf).output().unwrap();
    assert!(status.status.success());
    let woff2 = d.join("t.woff2");
    let a = text(&scat(&["-f", "--color=never"], &[&ttf]));
    let b = text(&scat(&["-f", "--color=never"], &[&woff2]));
    assert!(squeeze(&b).contains("format WOFF2\n"), "{b}");
    assert_eq!(
        a.lines()
            .filter(|l| l.contains("glyphs") || l.contains("family"))
            .collect::<Vec<_>>(),
        b.lines()
            .filter(|l| l.contains("glyphs") || l.contains("family"))
            .collect::<Vec<_>>()
    );
    let out = text(&scat(
        &["-f", "--protocol=blocks", "--color=always"],
        &[&woff2],
    ));
    assert!(out.contains('▀'), "{out}");
}

fn which(name: &str) -> Option<PathBuf> {
    std::env::var_os("PATH")?
        .to_str()?
        .split(':')
        .map(|d| Path::new(d).join(name))
        .find(|p| p.is_file())
}

#[test]
fn piped_output_is_the_file() {
    let d = dir("piped");
    let path = d.join("t.ttf");
    let font = tiny_font();
    std::fs::write(&path, &font).unwrap();
    let out = scat(&[], &[&path]);
    assert!(out.status.success());
    assert_eq!(out.stdout, font);
}

#[test]
fn corrupt_font_is_a_clean_error_and_scat_goes_on() {
    let d = dir("corrupt");
    let bad = d.join("bad.ttf");
    let mut font = tiny_font();
    font.truncate(200);
    std::fs::write(&bad, &font).unwrap();
    let bad_woff = d.join("bad.woff2");
    std::fs::write(&bad_woff, b"wOF2\0\x01\0\0garbage").unwrap();
    let good = d.join("good.txt");
    std::fs::write(&good, "still here\n").unwrap();
    let out = scat(&["-f", "--color=never"], &[&bad, &bad_woff, &good]);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("bad.ttf: cannot read font"), "{stderr}");
    assert!(stderr.contains("bad.woff2: cannot read font"), "{stderr}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("still here"), "{stdout}");
    assert!(!stdout.contains("family"), "{stdout}");
}

#[test]
fn directory_tiles_get_a_font_thumbnail() {
    let d = dir("tiles");
    let fonts = d.join("fonts");
    std::fs::create_dir_all(&fonts).unwrap();
    std::fs::write(fonts.join("ScatTest-Bold.ttf"), tiny_font()).unwrap();
    let cache = d.join("cache");
    let out = Command::new(SCAT)
        .args([
            "-f",
            "--style",
            "tiles",
            "--protocol=blocks",
            "--color=always",
        ])
        .arg(&fonts)
        .env("COLUMNS", "100")
        .env("LC_ALL", "C.UTF-8")
        .env("SCAT_CONFIG", "/nonexistent")
        .env("XDG_CACHE_HOME", &cache)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let thumbs: Vec<_> = std::fs::read_dir(cache.join("scat").join("thumbnails"))
        .unwrap()
        .map(|e| e.unwrap().metadata().unwrap().len())
        .collect();
    assert_eq!(thumbs.len(), 1);
    assert!(thumbs[0] > 0, "the font got no thumbnail");
}

#[test]
fn system_font_and_variable_axes() {
    let Some(font) = system_font(":", |p| p.ends_with(".ttf") || p.ends_with(".otf")) else {
        eprintln!("no system fonts; skipped");
        return;
    };
    let out = text(&scat(&["-f", "--color=never"], &[&font]));
    for key in ["family", "glyphs", "units/em", "coverage", "file size"] {
        assert!(out.contains(key), "missing {key} in\n{out}");
    }
    let Some(variable) = system_font(":variable=true", |p| p.ends_with(".ttf")) else {
        eprintln!("no variable system font; axis check skipped");
        return;
    };
    let out = text(&scat(&["-f", "--color=never"], &[&variable]));
    assert!(out.contains("  axes "), "{out}");
    assert!(out.contains(", default "), "{out}");
}
