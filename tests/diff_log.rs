//! Diff and log colouring through the binary, including streaming from standard input.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::mpsc;
use std::time::Duration;

const SCAT: &str = env!("CARGO_BIN_EXE_scat");

const DIFF: &str = "\
diff --git a/f.txt b/f.txt
index 111..222 100644
--- a/f.txt
+++ b/f.txt
@@ -1,3 +1,3 @@ fn x
 keep
--- gone
+++ new
\\ No newline at end of file
";

const LOG: &str = "\
2026-10-23T13:42:10Z INFO started
[2026-10-23 13:42:11] WARN slow
Oct 23 13:42:12 host ERROR boom
2026-10-23 13:42:13 DEBUG detail
2026-10-23 13:42:14 FATAL gone
plain line
";

fn dir() -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("diff_log");
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn command(args: &[&str]) -> Command {
    let d = dir();
    let mut cmd = Command::new(SCAT);
    cmd.args(args)
        .current_dir(&d)
        .env_remove("SCAT_CONFIG")
        .env_remove("SCAT_COLOR")
        .env_remove("XDG_CONFIG_HOME")
        .env("HOME", &d)
        .env("LC_ALL", "C.UTF-8");
    cmd
}

fn scat(name: &str, content: &str, args: &[&str]) -> Output {
    std::fs::write(dir().join(name), content).unwrap();
    command(args).arg(name).output().unwrap()
}

fn text(out: &Output) -> String {
    String::from_utf8(out.stdout.clone()).unwrap()
}

#[test]
fn diff_colours_exact() {
    let out = scat("a.diff", DIFF, &["-f", "--color=always", "-p"]);
    let expected = [
        "\x1b[1mdiff --git a/f.txt b/f.txt\x1b[0m",
        "\x1b[1mindex 111..222 100644\x1b[0m",
        "\x1b[1m--- a/f.txt\x1b[0m",
        "\x1b[1m+++ b/f.txt\x1b[0m",
        "\x1b[36m@@ -1,3 +1,3 @@ fn x\x1b[0m",
        " keep",
        // Inside a hunk these are a deletion and an addition, not file headers.
        "\x1b[31m--- gone\x1b[0m",
        "\x1b[32m+++ new\x1b[0m",
        "\x1b[2m\\ No newline at end of file\x1b[0m",
    ]
    .join("\n")
        + "\n";
    assert_eq!(text(&out), expected);
}

#[test]
fn diff_keeps_the_gutter() {
    let out = scat(
        "g.patch",
        "@@ -1 +1 @@\n-a\n+b\n",
        &["-f", "--color=always"],
    );
    assert_eq!(
        text(&out),
        "\x1b[2m   1 │\x1b[22m \x1b[36m@@ -1 +1 @@\x1b[0m\n\
         \x1b[2m   2 │\x1b[22m \x1b[31m-a\x1b[0m\n\
         \x1b[2m   3 │\x1b[22m \x1b[32m+b\x1b[0m\n"
    );
}

#[test]
fn diff_is_found_by_content() {
    let out = scat("changes.txt", DIFF, &["-f", "--color=always", "-p"]);
    assert!(text(&out).contains("\x1b[36m@@"), "{:?}", text(&out));
}

#[test]
fn diff_without_colour_is_plain() {
    let out = scat("n.diff", DIFF, &["-f", "--color=never", "-p"]);
    assert_eq!(text(&out), DIFF);
}

#[test]
fn piped_diff_and_log_are_cat() {
    assert_eq!(scat("p.diff", DIFF, &[]).stdout, DIFF.as_bytes());
    assert_eq!(scat("p.log", LOG, &[]).stdout, LOG.as_bytes());
}

#[test]
fn log_levels_and_timestamps() {
    let out = scat("a.log", LOG, &["-f", "--color=always", "-p"]);
    let expected = "\
\x1b[2m2026-10-23T13:42:10Z\x1b[0m \x1b[32mINFO\x1b[0m started
\x1b[2m[2026-10-23 13:42:11]\x1b[0m \x1b[33mWARN\x1b[0m slow
\x1b[2mOct 23 13:42:12\x1b[0m host \x1b[31mERROR\x1b[0m boom
\x1b[2m2026-10-23 13:42:13\x1b[0m \x1b[2mDEBUG\x1b[0m detail
\x1b[2m2026-10-23 13:42:14\x1b[0m \x1b[1;31mFATAL\x1b[0m gone
plain line
";
    assert_eq!(text(&out), expected);
}

#[test]
fn log_is_found_by_content() {
    let out = scat("noext", LOG, &["-f", "--color=always", "-p"]);
    assert!(text(&out).contains("\x1b[31mERROR\x1b[0m"));
}

#[test]
fn log_json_lines_only_colour_the_level() {
    let line = "{\"ts\":\"2026-10-23T13:42:10Z\",\"level\":\"error\",\"msg\":\"ERROR text\"}\n";
    let out = scat("j.log", line, &["-f", "--color=always", "-p"]);
    assert_eq!(
        text(&out),
        "{\"ts\":\"2026-10-23T13:42:10Z\",\"level\":\"\x1b[31merror\x1b[0m\",\"msg\":\"ERROR text\"}\n"
    );
}

#[test]
fn log_keeps_existing_colours_and_escapes_the_rest() {
    let out = scat(
        "c.log",
        "2026-10-23 13:42:10 \x1b[31mred\x1b[0m \x1b]0;evil\x07\n",
        &["-f", "--color=always", "-p"],
    );
    let got = text(&out);
    assert!(got.contains("\x1b[31mred"), "{got:?}");
    assert!(got.contains("^[]0;evil^G"), "{got:?}");
    assert!(!got.contains("\x1b]0;evil"));
}

#[test]
fn log_streams_line_by_line_from_stdin() {
    let mut child = command(&["-f", "--color=always", "-p", "-l", "log"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
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

    // Not a timestamp-first line: only -l log makes this a log.
    stdin.write_all(b"first ERROR line\n").unwrap();
    stdin.flush().unwrap();
    let expected = b"first \x1b[31mERROR\x1b[0m line\n";
    let mut got = Vec::new();
    while got.len() < expected.len() {
        match rx.recv_timeout(Duration::from_secs(5)) {
            Ok(chunk) => got.extend_from_slice(&chunk),
            Err(_) => break,
        }
    }
    assert_eq!(got, expected, "line not shown before EOF");

    drop(stdin);
    child.wait().unwrap();
    reader.join().unwrap();
}
