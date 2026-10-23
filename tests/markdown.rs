//! Markdown rendering through the binary: layout with `-f --color=never`, and raw bytes
//! when piped without `-f`.

use std::path::PathBuf;
use std::process::{Command, Output};

const SCAT: &str = env!("CARGO_BIN_EXE_scat");

const DOC: &str = "\
# Title

A paragraph that is long enough to wrap at sixty columns, with *emphasis*, \
**strong** and `code`.

- first item that also wraps because it keeps going for quite a while
  - nested
- [x] done

| Name | Size |
|:-----|-----:|
| a    |    1 |
| bb   |   22 |

> quoted \x1b]0;evil\x07 text

```sh
echo hi
```

See [the docs](https://example.com/docs).
";

const EXPECTED: &str = "\
Title
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

A paragraph that is long enough to wrap at sixty columns,
with emphasis, strong and `code`.

• first item that also wraps because it keeps going for
  quite a while
  ◦ nested
☑ done

┌──────┬──────┐
│ Name │ Size │
├──────┼──────┤
│ a    │    1 │
│ bb   │   22 │
└──────┴──────┘

│ quoted ^[]0;evil^G text

    echo hi

See the docs (https://example.com/docs).
";

fn fixture() -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("markdown");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("x.md");
    // Tests run in parallel: write a copy and rename it, so no run sees a half-written file.
    let tmp = dir.join(format!("x.md.{:?}", std::thread::current().id()));
    std::fs::write(&tmp, DOC).unwrap();
    std::fs::rename(&tmp, &path).unwrap();
    path
}

fn scat(args: &[&str]) -> Output {
    let path = fixture();
    let dir = path.parent().unwrap();
    Command::new(SCAT)
        .args(args)
        .arg(&path)
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

#[test]
fn renders_layout_without_colours() {
    let out = scat(&["-f", "--color=never"]);
    assert!(out.status.success());
    assert_eq!(String::from_utf8(out.stdout).unwrap(), EXPECTED);
}

#[test]
fn colours_never_leak_document_escapes() {
    let out = scat(&["-f", "--color=always"]);
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.contains("\x1b[0;1;95mTitle"), "{text:?}");
    assert!(!text.contains("\x1b]0;evil"), "{text:?}");
    assert!(!text.contains('\x07'));
}

#[test]
fn piped_output_is_the_file() {
    let out = scat(&[]);
    assert!(out.status.success());
    assert_eq!(out.stdout, DOC.as_bytes());
}

#[test]
fn local_images_are_drawn_and_remote_ones_are_not() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("markdown-images");
    std::fs::create_dir_all(dir.join("img")).unwrap();
    image::RgbaImage::from_pixel(16, 16, image::Rgba([0, 200, 0, 255]))
        .save(dir.join("img/dot one.png"))
        .unwrap();
    std::fs::write(
        dir.join("doc.md"),
        "Before.\n\n![dot](img/dot%20one.png)\n\n\
         ![web](https://example.com/dot.png) ![gone](img/missing.png)\n",
    )
    .unwrap();
    let run = |color: &str| {
        let out = Command::new(SCAT)
            .args(["-f", "--protocol=blocks", color, "doc.md"])
            .current_dir(&dir)
            .env_remove("SCAT_CONFIG")
            .env_remove("SCAT_PROTOCOL")
            .env_remove("XDG_CONFIG_HOME")
            .env("HOME", &dir)
            .env("LC_ALL", "C.UTF-8")
            .env("COLORTERM", "truecolor")
            .env("COLUMNS", "60")
            .output()
            .unwrap();
        assert!(out.status.success());
        String::from_utf8(out.stdout).unwrap()
    };

    let text = run("--color=always");
    // A 16x16 image: 16 columns of half blocks, 8 rows, not blown up to 60 columns.
    let rows: Vec<&str> = text.lines().filter(|l| l.contains('▀')).collect();
    assert_eq!(rows.len(), 8, "{text}");
    assert_eq!(rows[0].matches('▀').count(), 16, "{text}");
    assert!(rows[0].contains("\x1b[38;2;0;200;0"), "{text:?}");
    assert!(!text.contains("[image: dot]"), "{text}");
    assert!(text.contains("[image: web]"), "{text}");
    assert!(text.contains("[image: gone]"), "{text}");

    // Without colours the output is ASCII, and images keep their placeholder.
    let text = run("--color=never");
    assert!(text.contains("[image: dot]"), "{text}");
}
