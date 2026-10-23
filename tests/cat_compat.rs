//! scat must produce byte-identical output to GNU cat when stdout is not a terminal.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const SCAT: &str = env!("CARGO_BIN_EXE_scat");

/// Writes the fixtures once: tests run in parallel, and rewriting a file another test
/// is reading would truncate it under that test.
fn fixtures() -> PathBuf {
    static DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    DIR.get_or_init(write_fixtures).clone()
}

fn write_fixtures() -> PathBuf {
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

/// Runs the same arguments through GNU cat and scat in `dir` and compares stdout.
fn assert_same_as_cat(args: &[&str], dir: &Path) {
    let expected = run("cat", args, dir);
    let actual = run(SCAT, args, dir);
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

#[test]
fn show_ends_marks_carriage_returns_split_across_reads_and_files() {
    if !cat_available() {
        eprintln!("GNU cat not found, skipping");
        return;
    }
    let dir = fixtures();
    // 65535 bytes, then "\r\n": the CR ends the first 64 KiB read, the LF starts the next.
    let mut big = vec![b'x'; 65535];
    big.extend_from_slice(b"\r\nnext\r\n");
    std::fs::write(dir.join("cr-boundary.txt"), &big).unwrap();
    std::fs::write(dir.join("ends-cr.txt"), b"one\r").unwrap();
    std::fs::write(dir.join("starts-lf.txt"), b"\ntwo\n").unwrap();
    std::fs::write(dir.join("starts-x.txt"), b"x\n").unwrap();
    for flags in [&["-E"][..], &["-nE"], &["-bsE"], &["-A"]] {
        for files in [
            &["cr-boundary.txt"][..],
            &["ends-cr.txt", "starts-lf.txt"],
            &["ends-cr.txt", "starts-x.txt"],
            &["ends-cr.txt"],
            &["ends-cr.txt", "empty.txt", "starts-lf.txt"],
        ] {
            let mut args = flags.to_vec();
            args.extend_from_slice(files);
            assert_same_as_cat(&args, &dir);
        }
    }
}

#[test]
fn repeated_flags_are_accepted() {
    if !cat_available() {
        eprintln!("GNU cat not found, skipping");
        return;
    }
    let dir = fixtures();
    for flags in [
        &["-nn"][..],
        &["-EE"],
        &["-n", "-n"],
        &["-ss", "-bb"],
        &["-AA"],
    ] {
        let mut args = flags.to_vec();
        args.push("blank.txt");
        assert_same_as_cat(&args, &dir);
    }
}

#[test]
fn partial_lines_are_shown_before_the_newline() {
    use std::io::{Read, Write};
    use std::sync::mpsc;
    use std::time::Duration;

    let mut child = Command::new(SCAT)
        .arg("-n")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = child.stdout.take().unwrap();
    let (tx, rx) = mpsc::channel();
    let reader = std::thread::spawn(move || {
        let mut buf = [0u8; 256];
        while let Ok(n) = stdout.read(&mut buf) {
            if n == 0 || tx.send(buf[..n].to_vec()).is_err() {
                break;
            }
        }
    });

    stdin.write_all(b"partial").unwrap();
    stdin.flush().unwrap();
    let expected = b"     1\tpartial";
    let mut got = Vec::new();
    while got.len() < expected.len() {
        match rx.recv_timeout(Duration::from_secs(5)) {
            Ok(chunk) => got.extend_from_slice(&chunk),
            Err(_) => break,
        }
    }
    assert_eq!(got, expected, "partial line not written before EOF");

    stdin.write_all(b" line\nnext\n").unwrap();
    drop(stdin);
    child.wait().unwrap();
    reader.join().unwrap();
    got.extend(rx.try_iter().flatten());
    assert_eq!(got, b"     1\tpartial line\n     2\tnext\n");
}

#[test]
fn input_file_is_output_file_follows_gnu() {
    use std::fs::OpenOptions;
    use std::io::{Seek, SeekFrom};

    let dir = fixtures();
    let path = dir.join("self.txt");
    // (stdin at end, stdout O_APPEND, stdout at end, expect error), as GNU cat 9.12 does.
    let cases = [
        (true, false, true, false),
        (false, false, true, true),
        (true, true, false, false),
        (false, true, false, true),
        (false, false, false, false),
    ];
    for (in_end, append, out_end, error) in cases {
        std::fs::write(&path, b"abc\n").unwrap();
        let mut input = std::fs::File::open(&path).unwrap();
        if in_end {
            input.seek(SeekFrom::End(0)).unwrap();
        }
        let mut output = OpenOptions::new()
            .write(true)
            .append(append)
            .open(&path)
            .unwrap();
        if out_end {
            output.seek(SeekFrom::End(0)).unwrap();
        }
        let out = Command::new(SCAT)
            .stdin(input)
            .stdout(output)
            .stderr(std::process::Stdio::piped())
            .output()
            .unwrap();
        let case = (in_end, append, out_end);
        assert_eq!(out.status.code(), Some(i32::from(error)), "{case:?}");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(
            stderr.contains("input file is output file"),
            error,
            "{case:?}: {stderr}"
        );
        assert_eq!(std::fs::read(&path).unwrap(), b"abc\n", "{case:?}");
    }
}

#[test]
fn file_names_in_messages_are_sanitized() {
    let dir = fixtures();
    let name = "evil\x1b]52;c;aGk=\x07\u{202e}.txt";
    let out = run(SCAT, &[name], &dir);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains('\x1b') && !stderr.contains('\x07'),
        "{stderr:?}"
    );
    assert_eq!(
        stderr,
        "scat: evil^[]52;c;aGk=^G<U+202E>.txt: No such file or directory\n"
    );
}
