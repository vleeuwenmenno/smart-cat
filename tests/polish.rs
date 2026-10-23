//! Completions, man page, hex dump and the rules that keep piped output plain.

use std::process::{Command, Output};

const SCAT: &str = env!("CARGO_BIN_EXE_scat");

fn scat(args: &[&str], dir: &std::path::Path) -> Output {
    Command::new(SCAT)
        .args(args)
        .current_dir(dir)
        .env("NO_COLOR", "1")
        .env("LC_ALL", "C.UTF-8")
        .output()
        .unwrap()
}

fn temp_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("scat-polish-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn completions_are_printed_for_each_shell() {
    let dir = temp_dir("completions");
    for shell in ["fish", "bash", "zsh"] {
        let out = scat(&["--completions", shell], &dir);
        assert!(out.status.success(), "{shell}");
        let text = String::from_utf8(out.stdout).unwrap();
        assert!(text.contains("--line-numbers") || text.contains("line-numbers"));
    }
}

#[test]
fn man_page_has_a_title_line() {
    let dir = temp_dir("man");
    let out = scat(&["--man"], &dir);
    assert!(out.status.success());
    assert!(String::from_utf8(out.stdout).unwrap().contains(".TH"));
}

#[test]
fn binary_files_render_as_a_hex_dump_and_respect_max_bytes() {
    let dir = temp_dir("hex");
    std::fs::write(dir.join("b.bin"), (0..=255u8).collect::<Vec<_>>()).unwrap();
    std::fs::write(dir.join("c.toml"), "[hexdump]\nmax_bytes = 32\n").unwrap();
    let out = scat(&["-f", "--config", "c.toml", "b.bin"], &dir);
    let text = String::from_utf8(out.stdout).unwrap();
    assert_eq!(text.lines().count(), 3, "{text}");
    assert!(text.starts_with("00000000  00 01 02 03"));
    assert!(
        text.ends_with("… 224 B more, use --raw for bytes\n"),
        "{text}"
    );

    // Piped, without -f, the bytes are the file.
    let piped = scat(&["b.bin"], &dir);
    assert_eq!(piped.stdout, (0..=255u8).collect::<Vec<_>>());
}

#[test]
fn no_headers_or_pager_when_not_a_terminal() {
    let dir = temp_dir("headers");
    std::fs::write(dir.join("a.txt"), "one\n").unwrap();
    let out = Command::new(SCAT)
        .args(["-f", "--paging=always", "-p", "a.txt", "a.txt"])
        .current_dir(&dir)
        .env("NO_COLOR", "1")
        .env("SCAT_PAGER", "/nonexistent/pager")
        .output()
        .unwrap();
    assert_eq!(out.stdout, b"one\none\n");
    assert!(out.stderr.is_empty());
}

#[test]
fn install_completions_writes_and_removes_user_files() {
    let home = std::env::temp_dir().join(format!("scat-completions-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).unwrap();
    let scat = |arg: &str| {
        std::process::Command::new(env!("CARGO_BIN_EXE_scat"))
            .arg(arg)
            .env("HOME", &home)
            .env_remove("XDG_CONFIG_HOME")
            .env_remove("XDG_DATA_HOME")
            .output()
            .unwrap()
    };
    let man = home.join(".local/share/man/man1/scat.1");

    let out = scat("--install-completions");
    assert!(out.status.success(), "{out:?}");
    assert!(std::fs::read_to_string(&man).unwrap().contains(".TH"));

    std::fs::write(&man, "old").unwrap();
    assert!(scat("--refresh-completions").status.success());
    assert!(std::fs::read_to_string(&man).unwrap().contains(".TH"));

    assert!(scat("--uninstall-completions").status.success());
    assert!(!man.exists());
    let _ = std::fs::remove_dir_all(&home);
}
