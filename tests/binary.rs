//! Binary summaries: system executables and libraries when present, a hand-built
//! WebAssembly module, the hex fallback, and piped output staying the file's bytes.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const SCAT: &str = env!("CARGO_BIN_EXE_scat");

fn fixture(name: &str, data: &[u8]) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("binary");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    std::fs::write(&path, data).unwrap();
    path
}

fn scat(args: &[&str], dir: &Path) -> Output {
    Command::new(SCAT)
        .args(args)
        .current_dir(dir)
        .env("HOME", dir)
        .env_remove("SCAT_CONFIG")
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("SCAT_COLOR")
        .env("LC_ALL", "C.UTF-8")
        .output()
        .unwrap()
}

fn render(path: &Path) -> String {
    let out = scat(
        &[
            "-f",
            "--color=never",
            path.file_name().unwrap().to_str().unwrap(),
        ],
        path.parent().unwrap(),
    );
    assert!(out.status.success(), "{out:?}");
    String::from_utf8(out.stdout).unwrap()
}

fn has_row(text: &str, key: &str) -> bool {
    text.lines().any(|l| l.trim_start().starts_with(key))
}

/// A WebAssembly module: imports `env.f0`..`env.f6`, exports `run`, has one memory and a
/// custom section whose name carries an escape sequence.
fn wasm() -> Vec<u8> {
    let mut m = b"\0asm\x01\0\0\0".to_vec();
    m.extend([1, 4, 1, 0x60, 0, 0]); // type () -> ()
    let mut imports = vec![7u8];
    for i in 0..7u8 {
        imports.extend([3, b'e', b'n', b'v', 2, b'f', b'0' + i, 0, 0]);
    }
    m.extend([2, imports.len() as u8]);
    m.extend(&imports);
    m.extend([3, 2, 1, 0]); // one function
    m.extend([5, 3, 1, 0, 2]); // memory: min 2, no max
    m.extend([7, 7, 1, 3, b'r', b'u', b'n', 0, 7]); // export "run" = func 7
    m.extend([10, 4, 1, 2, 0, 0x0b]); // code
    let name = b"x\x1b[2Jy";
    m.extend([0, (name.len() + 1 + 2) as u8, name.len() as u8]);
    m.extend(name);
    m.extend([1, 2]);
    m
}

#[test]
fn wasm_summary_and_hex() {
    let path = fixture("tiny.wasm", &wasm());
    let text = render(&path);
    assert!(
        text.starts_with("tiny.wasm  WebAssembly module\n"),
        "{text}"
    );
    for row in [
        "version          1",
        "imports          7",
        "exports          1",
        "memory           2 pages min, no max",
    ] {
        assert!(text.lines().any(|l| l.trim() == row), "{row:?} in\n{text}");
    }
    // Five names are shown, the rest counted.
    assert!(
        text.contains("env.f0, env.f1, env.f2, env.f3, env.f4, … 2 more"),
        "{text}"
    );
    assert!(text.contains("x^[[2Jy"), "{text}");
    assert!(!text.contains('\x1b'));
    assert!(text.contains("00000000  00 61 73 6d 01 00 00 00"), "{text}");
}

#[test]
fn piped_output_is_the_input() {
    let bytes = wasm();
    let path = fixture("piped.wasm", &bytes);
    let out = scat(&["piped.wasm"], path.parent().unwrap());
    assert!(out.status.success());
    assert_eq!(out.stdout, bytes);
}

#[test]
fn truncated_elf_falls_back_to_hex() {
    let path = fixture(
        "short.elf",
        b"\x7fELF\x02\x01\x01\0\0\0\0\0\0\0\0\0\x03\0\x3e\0",
    );
    let text = render(&path);
    assert!(
        text.starts_with("00000000  7f 45 4c 46 02 01 01 00"),
        "{text}"
    );
    assert!(!text.contains("format"), "{text}");
}

#[test]
fn broken_wasm_falls_back_to_hex() {
    let path = fixture("broken.wasm", b"\0asm\x01\0\0\0\x01\xff\xff");
    let text = render(&path);
    assert!(text.starts_with("00000000  00 61 73 6d"), "{text}");
}

#[test]
fn rule_can_pick_hex_over_the_summary() {
    let path = fixture("rule.wasm", &wasm());
    let dir = path.parent().unwrap();
    let config = dir.join("rule.toml");
    std::fs::write(
        &config,
        "[[rule]]\nmatch = { ext = \"wasm\" }\nrenderer = \"hex\"\n",
    )
    .unwrap();
    let out = Command::new(SCAT)
        .args(["-f", "--color=never", "rule.wasm"])
        .current_dir(dir)
        .env("HOME", dir)
        .env("SCAT_CONFIG", &config)
        .env("LC_ALL", "C.UTF-8")
        .output()
        .unwrap();
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.starts_with("00000000  00 61 73 6d"), "{text}");
}

#[test]
fn system_executable() {
    let Some(path) = ["/bin/sh", "/usr/bin/sh", "/bin/ls"]
        .iter()
        .map(PathBuf::from)
        .find(|p| p.is_file())
    else {
        return;
    };
    let Ok(bytes) = std::fs::read(&path) else {
        return;
    };
    if !bytes.starts_with(b"\x7fELF") {
        return;
    }
    let text = render(&path);
    for key in ["format", "type", "architecture", "sections", "stripped"] {
        assert!(has_row(&text, key), "{key} missing in\n{text}");
    }
    assert!(text.contains("ELF "), "{text}");
    assert!(text.contains("00000000  7f 45 4c 46"), "{text}");
    // The dump is the short one (256 bytes = 16 lines), not 512.
    assert!(!text.contains("00000100  "), "{text}");
}

#[test]
fn system_shared_library_and_archive() {
    let mut found_so = false;
    let mut found_a = false;
    for dir in [
        "/usr/lib",
        "/usr/lib64",
        "/usr/lib/x86_64-linux-gnu",
        "/lib64",
    ] {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            let Ok(meta) = std::fs::metadata(&path) else {
                continue;
            };
            if !meta.is_file() || meta.len() > 8 << 20 || meta.len() < 4096 {
                continue;
            }
            if !found_so && name.starts_with("lib") && name.contains(".so") {
                let head = std::fs::read(&path).unwrap_or_default();
                if head.starts_with(b"\x7fELF") {
                    let text = render(&path);
                    assert!(text.contains("shared library"), "{name}\n{text}");
                    assert!(
                        has_row(&text, "soname") || has_row(&text, "needed"),
                        "{text}"
                    );
                    found_so = true;
                }
            }
            if !found_a && name.starts_with("lib") && name.ends_with(".a") {
                let head = std::fs::read(&path).unwrap_or_default();
                // A GNU archive with a symbol table (the empty glibc stubs have none).
                if head.starts_with(b"!<arch>\n/") {
                    let text = render(&path);
                    assert!(text.contains("ar archive"), "{name}\n{text}");
                    assert!(has_row(&text, "members"), "{text}");
                    found_a = true;
                }
            }
        }
    }
}
