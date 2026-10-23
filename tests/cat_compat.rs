//! scat must produce byte-identical output to GNU cat when stdout is not a terminal.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const SCAT: &str = env!("CARGO_BIN_EXE_scat");

fn fixtures() -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("cat-compat");
    std::fs::create_dir_all(&dir).unwrap();
    let files: &[(&str, &[u8])] = &[
        ("plain.txt", b"hello\nworld\n"),
        ("blank.txt", b"\n\n\na\n\n\n\nb\n\n"),
        ("no-newline.txt", b"first\nsecond"),
        ("control.txt", b"A line of ordinary text so the file is detected as text.\ntab\there\x1b[31mred\x1b[0m\r\n\x7f\x80\xff\xe9 caf\xc3\xa9\n"),
        ("code.rs", b"fn main() {\n\tprintln!(\"hi\");\n}\n"),
        ("binary.bin", b"\x00\x01\x02\x89PNG\x00\n\x00\xff"),
        ("empty.txt", b""),
    ];
    for (name, data) in files {
        std::fs::write(dir.join(name), data).unwrap();
    }
    dir
}

fn run(program: &str, args: &[&str], dir: &Path) -> Output {
    Command::new(program)
        .args(args)
        .current_dir(dir)
        .env_remove("SCAT_CONFIG")
        .env("HOME", dir)
        .env_remove("XDG_CONFIG_HOME")
        .env("LC_ALL", "C")
        .output()
        .unwrap()
}

fn cat_available() -> bool {
    Command::new("cat")
        .arg("--version")
        .output()
        .is_ok_and(|o| {
            o.status.success() && String::from_utf8_lossy(&o.stdout).contains("GNU coreutils")
        })
}

#[test]
fn output_matches_gnu_cat() {
    if !cat_available() {
        eprintln!("GNU cat not found, skipping");
        return;
    }
    let dir = fixtures();
    let all = [
        "plain.txt",
        "blank.txt",
        "no-newline.txt",
        "control.txt",
        "code.rs",
        "binary.bin",
        "empty.txt",
    ];
    let flag_sets: &[&[&str]] = &[
        &[],
        &["-n"],
        &["-b"],
        &["-s"],
        &["-E"],
        &["-T"],
        &["-v"],
        &["-A"],
        &["-e"],
        &["-t"],
        &["-ns"],
        &["-bs"],
        &["-nA"],
        &["-u"],
    ];
    for flags in flag_sets {
        for file in all {
            let mut args = flags.to_vec();
            args.push(file);
            let expected = run("cat", &args, &dir);
            let actual = run(SCAT, &args, &dir);
            assert_eq!(
                actual.stdout, expected.stdout,
                "stdout differs for {args:?}"
            );
            assert_eq!(
                actual.status.code(),
                expected.status.code(),
                "status for {args:?}"
            );
        }
        // Numbering and squeezing continue across files.
        let mut args = flags.to_vec();
        args.extend(["no-newline.txt", "blank.txt", "plain.txt"]);
        let expected = run("cat", &args, &dir);
        let actual = run(SCAT, &args, &dir);
        assert_eq!(
            actual.stdout, expected.stdout,
            "stdout differs for {args:?}"
        );
    }
}

#[test]
fn missing_files_are_reported_and_skipped() {
    let dir = fixtures();
    let out = run(SCAT, &["plain.txt", "missing.txt", "plain.txt"], &dir);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(out.stdout, b"hello\nworld\nhello\nworld\n");
    assert_eq!(
        String::from_utf8_lossy(&out.stderr),
        "scat: missing.txt: No such file or directory\n"
    );
}

#[test]
fn directories_are_an_error() {
    let dir = fixtures();
    let out = run(SCAT, &["."], &dir);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(
        String::from_utf8_lossy(&out.stderr),
        "scat: .: Is a directory\n"
    );
}

#[test]
fn reads_stdin() {
    use std::io::Write;
    let mut child = Command::new(SCAT)
        .args(["-n", "-"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(b"a\nb\n").unwrap();
    let out = child.wait_with_output().unwrap();
    assert_eq!(out.stdout, b"     1\ta\n     2\tb\n");
}

#[test]
fn force_render_highlights_and_numbers() {
    let dir = fixtures();
    let out = run(
        SCAT,
        &["-f", "--color=always", "--theme=ansi", "-n", "code.rs"],
        &dir,
    );
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.contains("\x1b["), "expected colour codes: {text:?}");
    assert!(
        text.contains("   1 "),
        "expected a line number gutter: {text:?}"
    );
    assert!(!text.contains('\t'), "tabs should be expanded: {text:?}");
}

#[test]
fn force_render_sanitizes_control_characters() {
    let dir = fixtures();
    let out = run(SCAT, &["-f", "--color=never", "control.txt"], &dir);
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(
        !text.contains('\x1b'),
        "escape characters must not pass: {text:?}"
    );
    assert!(text.contains("^?"), "{text:?}");
}

#[test]
fn force_render_skips_binary_files() {
    let dir = fixtures();
    let out = run(SCAT, &["-f", "binary.bin"], &dir);
    assert!(out.stdout.is_empty());
    assert!(String::from_utf8_lossy(&out.stderr).contains("binary file"));
    assert_eq!(out.status.code(), Some(0));
}

#[test]
fn config_rule_can_force_raw() {
    let dir = fixtures();
    let config = dir.join("raw.toml");
    std::fs::write(
        &config,
        "[[rule]]\nmatch = { ext = \"rs\" }\nrenderer = \"raw\"\n",
    )
    .unwrap();
    let out = run(
        SCAT,
        &[
            "-f",
            "--color=always",
            "--config",
            config.to_str().unwrap(),
            "code.rs",
        ],
        &dir,
    );
    assert_eq!(out.stdout, std::fs::read(dir.join("code.rs")).unwrap());
}

#[test]
fn broken_config_warns_but_still_prints() {
    let dir = fixtures();
    let config = dir.join("broken.toml");
    std::fs::write(&config, "[defaults]\nnot_a_key = 1\n").unwrap();
    let out = run(
        SCAT,
        &["--config", config.to_str().unwrap(), "plain.txt"],
        &dir,
    );
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(out.stdout, b"hello\nworld\n");
    assert!(String::from_utf8_lossy(&out.stderr).contains("using defaults"));
}

#[test]
fn line_numbers_are_on_by_default_when_rendering() {
    let dir = fixtures();
    let render = |extra: &[&str]| {
        let mut args = vec!["-f", "--color=never"];
        args.extend_from_slice(extra);
        args.push("plain.txt");
        String::from_utf8(run(SCAT, &args, &dir).stdout).unwrap()
    };
    assert_eq!(render(&[]), "   1 | hello\n   2 | world\n");
    assert_eq!(render(&["-p"]), "hello\nworld\n");

    let config = dir.join("no-numbers.toml");
    std::fs::write(&config, "[defaults]\nline_numbers = false\n").unwrap();
    let config = config.to_str().unwrap();
    assert_eq!(render(&["--config", config]), "hello\nworld\n");
    assert_eq!(
        render(&["--config", config, "-n"]),
        "   1 | hello\n   2 | world\n"
    );
}
