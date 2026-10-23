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
    std::fs::write(&path, DOC).unwrap();
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
