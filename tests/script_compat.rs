//! Shell idioms that scripts use with cat, run with `sh -c` (and `bash -c` where a
//! bash feature is needed) once with GNU cat and once with scat as `$CAT`. Standard
//! output and exit status must be identical, and so must standard error once the
//! program name is taken out.

use std::path::{Path, PathBuf};
use std::process::Command;

const SCAT: &str = env!("CARGO_BIN_EXE_scat");

fn gnu_cat() -> bool {
    Command::new("cat")
        .arg("--version")
        .output()
        .is_ok_and(|o| {
            o.status.success() && String::from_utf8_lossy(&o.stdout).contains("GNU coreutils")
        })
}

/// Fixtures in a fresh directory per test, so tests that write files do not race.
fn fixtures(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("script-compat")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("dir")).unwrap();
    let files: &[(&str, &[u8])] = &[
        ("f", b"a\nb\n"),
        ("empty", b""),
        ("nonl", b"no newline"),
        ("nul", b"a\0b\0\n\0"),
        ("badutf", b"\xff\xfe bad \xc3\x28\n"),
        ("crlf", b"x\r\ny\r\n"),
        ("esc", b"\x1b[2J\x1b]0;title\x07art\n"),
        ("-dash", b"dash\n"),
    ];
    for (name, data) in files {
        std::fs::write(dir.join(name), data).unwrap();
    }
    // A few MiB of varied bytes: big enough for several kernel copy calls.
    let big: Vec<u8> = (0u32..3_000_000)
        .map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8)
        .collect();
    std::fs::write(dir.join("big"), big).unwrap();
    dir
}

struct Run {
    stdout: Vec<u8>,
    stderr: String,
    status: Option<i32>,
}

fn run(shell: &str, cat: &str, snippet: &str, dir: &Path) -> Run {
    let out = Command::new(shell)
        .arg("-c")
        .arg(snippet)
        .current_dir(dir)
        .env("CAT", cat)
        .env("HOME", dir)
        .env_remove("SCAT_CONFIG")
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("POSIXLY_CORRECT")
        .env("LC_ALL", "C")
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr)
        .lines()
        .map(|line| {
            let line = line
                .strip_prefix("scat: ")
                .or_else(|| line.strip_prefix("cat: "))
                .unwrap_or(line);
            line.replace("'scat --help'", "'cat --help'")
        })
        .collect::<Vec<_>>()
        .join("\n");
    Run {
        stdout: out.stdout,
        stderr,
        status: out.status.code(),
    }
}

/// Runs each snippet with GNU cat and scat and compares the results.
fn compare(name: &str, shell: &str, snippets: &[&str], check_stderr: bool) {
    if !gnu_cat() {
        eprintln!("GNU cat not found, skipping");
        return;
    }
    for snippet in snippets {
        // Fresh fixtures for each run: snippets append to and create files.
        let gnu = run(shell, "cat", snippet, &fixtures(name));
        let ours = run(shell, SCAT, snippet, &fixtures(name));
        assert_eq!(
            String::from_utf8_lossy(&gnu.stdout),
            String::from_utf8_lossy(&ours.stdout),
            "stdout differs for: {snippet}"
        );
        assert!(
            gnu.stdout == ours.stdout,
            "stdout bytes differ for: {snippet}"
        );
        assert_eq!(
            gnu.status, ours.status,
            "exit status differs for: {snippet}"
        );
        if check_stderr {
            assert_eq!(gnu.stderr, ours.stderr, "stderr differs for: {snippet}");
        }
    }
}

#[test]
fn posix_shell_idioms() {
    compare(
        "posix",
        "sh",
        &[
            r#"x=$($CAT f); printf '[%s]' "$x""#,
            "$CAT f nonl > out && $CAT out",
            "printf 'q\\n' | $CAT >> app; printf 'r\\n' | $CAT >> app; $CAT app",
            "$CAT <<EOF\nhere $HOME\nEOF",
            "$CAT <<'EOF' > hd\nlit $HOME\nEOF\n$CAT hd",
            r#"$CAT f | while read l; do echo "<$l>"; done"#,
            "echo in | $CAT -",
            "echo in | $CAT - f -",
            "echo in | $CAT /dev/stdin",
            "echo in | $CAT /proc/self/fd/0",
            "$CAT -- -dash",
            "$CAT f -- f",
            "$CAT /dev/null; echo st=$?",
            "$CAT < /dev/null",
            "$CAT empty nonl nul badutf crlf esc",
            "$CAT -u f",
            "$CAT f -n",
            "$CAT -n f nonl f",
            "$CAT -bsET f crlf nul",
            "mkfifo fifo; (printf viafifo > fifo &); $CAT fifo",
            "printf '%s\\n' f nonl | xargs $CAT",
            "find . -name f -exec $CAT {} +",
        ],
        true,
    );
}

/// The plain copy goes through copy_file_range, sendfile or splice depending on what
/// the input and output are; every combination must give cat's bytes.
#[test]
fn kernel_copy_paths() {
    compare(
        "copy",
        "sh",
        &[
            // file to pipe (sendfile), pipe to pipe (splice)
            "$CAT big | cksum",
            "$CAT < big | cksum",
            "cat big | $CAT | cksum",
            // file to file (copy_file_range), pipe to file (splice)
            "$CAT big > out; cksum out",
            "cat big | $CAT > out; cksum out",
            // O_APPEND output: the kernel refuses, the fallback copies
            "$CAT f > out; $CAT big >> out; cksum out",
            "$CAT f > out; cat big | $CAT >> out; cksum out",
            // several inputs, mixed kinds, into one output
            "echo mid | $CAT f - big empty nonl > out; cksum out",
            // /proc files report size 0
            "$CAT /proc/version > out; cmp out /proc/version && echo same",
            "$CAT /proc/version | cmp - /proc/version && echo same",
            // to /dev/null, and an early reader exit
            "$CAT big > /dev/null; echo st=$?",
            "$CAT big big big | head -c 5 | od -c",
            "head -c 100000 /dev/zero | $CAT | wc -c",
        ],
        true,
    );
}

#[test]
fn bash_idioms() {
    compare(
        "bash",
        "bash",
        &[
            "$CAT <(printf proc)",
            "$CAT <<< 'here string'",
            "set -o pipefail; $CAT big big big big | head -c 1 > /dev/null; echo st=$?",
            "set -o pipefail; $CAT f | head -1; echo st=$?",
        ],
        true,
    );
}

#[test]
fn errors_and_exit_status() {
    compare(
        "errors",
        "sh",
        &[
            "$CAT missing; echo st=$?",
            "$CAT f missing f; echo st=$?",
            "$CAT dir; echo st=$?",
            "$CAT f > f2; $CAT f2 >> f2; echo st=$?",
            "$CAT --bogus; echo st=$?",
            "$CAT -z; echo st=$?",
            "$CAT -dash; echo st=$?",
            "$CAT --help > /dev/null; echo st=$?",
            "$CAT --version > /dev/null; echo st=$?",
            // unique abbreviations of long options, as getopt allows
            "$CAT --show-a f",
            "$CAT --show-e f",
            "$CAT --squeeze f",
        ],
        true,
    );
    // The messages differ in wording (cat quotes the empty name, explains ambiguity);
    // what scripts see is the output and the status.
    compare(
        "errors-status",
        "sh",
        &["$CAT ''; echo st=$?", "$CAT --numb f; echo st=$?"],
        false,
    );
}
