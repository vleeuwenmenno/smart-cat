//! Directory listing behaviour of the scat binary.

use std::path::PathBuf;
use std::process::{Command, Output};

const SCAT: &str = env!("CARGO_BIN_EXE_scat");

fn tree(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    std::fs::create_dir_all(dir.join("Apple")).unwrap();
    std::fs::create_dir_all(dir.join(".hiddendir")).unwrap();
    for f in ["b.txt", "A2.txt", "a10.txt", ".secret"] {
        std::fs::write(dir.join(f), "x").unwrap();
    }
    dir
}

fn scat(args: &[&str], dir: &PathBuf) -> Output {
    Command::new(SCAT)
        .args(args)
        .current_dir(dir)
        .env_remove("SCAT_CONFIG")
        .env_remove("LS_COLORS")
        .env("HOME", dir)
        .env_remove("XDG_CONFIG_HOME")
        .env("COLUMNS", "80")
        .env("LC_ALL", "C")
        .output()
        .unwrap()
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn lists_dirs_first_and_hides_dotfiles() {
    let dir = tree("list-basic");
    let out = scat(&["-f", "--color=never", "."], &dir);
    assert!(out.status.success());
    assert_eq!(stdout(&out), "Apple/  sub/  A2.txt  a10.txt  b.txt\n");
}

#[test]
fn all_includes_hidden() {
    let dir = tree("list-all");
    let out = scat(&["-f", "--color=never", "--all", "."], &dir);
    let text = stdout(&out);
    assert!(text.contains(".hiddendir/"));
    assert!(text.contains(".secret"));
    assert!(text.find(".hiddendir/") < text.find("A2.txt"));
}

#[test]
fn narrow_terminal_uses_columns_top_to_bottom() {
    let dir = tree("list-narrow");
    let out = Command::new(SCAT)
        .args(["-f", "--color=never", "."])
        .current_dir(&dir)
        .env("COLUMNS", "20")
        .env("HOME", &dir)
        .output()
        .unwrap();
    assert_eq!(stdout(&out), "Apple/  a10.txt\nsub/    b.txt\nA2.txt\n");
}

#[test]
fn long_view_has_size_and_age() {
    let dir = tree("list-long");
    let out = scat(&["-f", "--color=never", "--long", "."], &dir);
    let text = stdout(&out);
    assert!(
        text.lines()
            .any(|l| l.starts_with("d ") && l.ends_with("Apple/"))
    );
    assert!(
        text.lines()
            .any(|l| l.contains("1 B") && l.ends_with("b.txt"))
    );
    assert!(text.contains("now"));
}

#[test]
fn symlinks_show_target_only_in_long_view() {
    let dir = tree("list-link");
    std::os::unix::fs::symlink("b.txt", dir.join("link")).unwrap();
    let grid = scat(&["-f", "--color=never", "."], &dir);
    assert!(!stdout(&grid).contains("->"));
    let long = scat(&["-f", "--color=never", "--long", "."], &dir);
    assert!(stdout(&long).contains("link -> b.txt"));
}

#[test]
fn colors_follow_ls_colors() {
    let dir = tree("list-colors");
    let out = Command::new(SCAT)
        .args(["-f", "--color=always", "."])
        .current_dir(&dir)
        .env("LS_COLORS", "di=01;35:*.txt=32")
        .env("COLUMNS", "80")
        .env("HOME", &dir)
        .output()
        .unwrap();
    let text = stdout(&out);
    assert!(text.contains("\x1b[01;35mApple/\x1b[0m"));
    assert!(text.contains("\x1b[32mb.txt\x1b[0m"));
}

#[test]
fn weird_names_are_sanitized() {
    let dir = tree("list-weird");
    std::fs::write(dir.join("evil\x1b]0;pwn\x07\nname"), "x").unwrap();
    for args in [&["-f", "."][..], &["-f", "--long", "."][..]] {
        let out = scat(args, &dir);
        let text = stdout(&out);
        assert!(!text.contains('\x1b'), "raw ESC in {text:?}");
        assert!(!text.contains('\x07'));
        assert!(text.contains("evil^[]0;pwn^G^Jname"), "{text:?}");
    }
}

#[test]
fn several_directories_get_headers() {
    let dir = tree("list-multi");
    std::fs::write(dir.join("sub").join("inner.txt"), "x").unwrap();
    let out = scat(&["-f", "--color=never", "sub", "Apple", "b.txt"], &dir);
    let text = stdout(&out);
    assert!(text.starts_with("sub:\ninner.txt\n\nApple:\n"), "{text:?}");
    assert!(text.ends_with(" x"), "{text:?}");
}

#[test]
fn unreadable_directory_reports_and_continues() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tree("list-denied");
    let locked = dir.join("locked");
    std::fs::create_dir(&locked).unwrap();
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
    if std::fs::read_dir(&locked).is_ok() {
        return; // running as root
    }
    let out = scat(&["-f", "--color=never", "locked", "sub"], &dir);
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(
        String::from_utf8_lossy(&out.stderr),
        "scat: locked: Permission denied\n"
    );
}

#[test]
fn piped_directory_is_still_an_error() {
    let dir = tree("list-piped");
    let out = scat(&["sub"], &dir);
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty());
    assert_eq!(
        String::from_utf8_lossy(&out.stderr),
        "scat: sub: Is a directory\n"
    );
}

#[test]
fn config_can_disable_directory_listing() {
    let dir = tree("list-config");
    let cfg = dir.join("cfg.toml");
    std::fs::write(&cfg, "[defaults]\ndirectories = \"error\"\n").unwrap();
    let out = scat(&["-f", "--config", cfg.to_str().unwrap(), "sub"], &dir);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(
        String::from_utf8_lossy(&out.stderr),
        "scat: sub: Is a directory\n"
    );
}
