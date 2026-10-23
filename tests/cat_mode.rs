//! Invoked as `cat` (a symlink), scat behaves like GNU cat even on a terminal, and the
//! paging rule that keeps scripts from blocking in a pager. Terminal tests run under
//! util-linux `script`, which gives the command a pseudo-terminal; they are skipped
//! when `script` is not available.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const SCAT: &str = env!("CARGO_BIN_EXE_scat");

/// A fresh directory with a `cat` symlink to scat and a few files.
fn setup(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("cat-mode")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("dir")).unwrap();
    std::os::unix::fs::symlink(SCAT, dir.join("cat")).unwrap();
    std::fs::write(dir.join("code.rs"), "fn main() {}\n").unwrap();
    let long: String = (1..=200).map(|n| format!("line {n}\n")).collect();
    std::fs::write(dir.join("long.txt"), long).unwrap();
    dir
}

fn command(program: &str, dir: &Path) -> Command {
    let mut cmd = Command::new(program);
    cmd.current_dir(dir)
        .env("HOME", dir)
        .env("XDG_CONFIG_HOME", dir.join("config"))
        .env_remove("SCAT_CONFIG")
        .env_remove("SCAT_PAGING")
        .env_remove("SCAT_THEME")
        .env_remove("XDG_RUNTIME_DIR")
        .env_remove("NO_COLOR")
        .env("TERM", "xterm-256color")
        .env("LC_ALL", "C.UTF-8");
    cmd
}

fn script_available() -> bool {
    Command::new("script")
        .args(["-qec", "true", "/dev/null"])
        .output()
        .is_ok_and(|o| o.status.success())
}

/// Runs a `sh` command line on a pseudo-terminal and returns what it printed. The
/// terminal turns `\n` into `\r\n`; that is undone here.
fn on_pty(dir: &Path, line: &str, env: &[(&str, &str)]) -> String {
    let mut cmd = command("script", dir);
    cmd.args(["-qec", line, "/dev/null"])
        .env("SHELL", "/bin/sh");
    for (name, value) in env {
        cmd.env(name, value);
    }
    let out = cmd.output().unwrap();
    String::from_utf8_lossy(&out.stdout).replace("\r\n", "\n")
}

fn run(program: &str, args: &[&str], dir: &Path) -> Output {
    command(program, dir).args(args).output().unwrap()
}

#[test]
fn raw_output_on_a_terminal() {
    if !script_available() {
        eprintln!("script not found, skipping");
        return;
    }
    let dir = setup("raw");
    assert_eq!(on_pty(&dir, "./cat code.rs", &[]), "fn main() {}\n");
    // The same binary under its own name renders: colours and a line number gutter.
    let scat = on_pty(&dir, &format!("'{SCAT}' code.rs"), &[]);
    assert!(scat.contains('\x1b'), "{scat:?}");
    assert!(scat.contains("1 │"), "{scat:?}");
    // No pager, even though the file is longer than the screen and paging is forced.
    let paged = on_pty(
        &dir,
        "./cat long.txt",
        &[
            ("SCAT_PAGER", "/bin/sed s/^/PAGED:/"),
            ("SCAT_PAGING", "always"),
        ],
    );
    assert!(!paged.contains("PAGED:"), "{paged:?}");
    assert!(paged.starts_with("line 1\nline 2\n"));
    // A directory is an error, as with cat.
    let listed = on_pty(&dir, "./cat dir; echo st=$?", &[]);
    assert_eq!(listed, "cat: dir: Is a directory\nst=1\n");
}

#[test]
fn scat_flags_still_work() {
    let dir = setup("flags");
    let out = run(&dir.join("cat").to_string_lossy(), &["-f", "code.rs"], &dir);
    assert!(String::from_utf8_lossy(&out.stdout).contains("1 │"));
    let out = run(&dir.join("cat").to_string_lossy(), &["--diagnose"], &dir);
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("invoked as: cat; strict mode"), "{text}");
}

#[test]
fn messages_use_the_cat_prefix() {
    let dir = setup("prefix");
    let cat = dir.join("cat");
    let out = run(&cat.to_string_lossy(), &["missing"], &dir);
    assert_eq!(
        String::from_utf8_lossy(&out.stderr),
        "cat: missing: No such file or directory\n"
    );
    assert_eq!(out.status.code(), Some(1));
    let out = run(&cat.to_string_lossy(), &["-z"], &dir);
    assert_eq!(
        String::from_utf8_lossy(&out.stderr),
        "cat: invalid option -- 'z'\nTry 'cat --help' for more information.\n"
    );
    let out = run(SCAT, &["missing"], &dir);
    assert_eq!(
        String::from_utf8_lossy(&out.stderr),
        "scat: missing: No such file or directory\n"
    );
}

#[test]
fn config_can_turn_rendering_back_on() {
    if !script_available() {
        eprintln!("script not found, skipping");
        return;
    }
    let dir = setup("render");
    std::fs::create_dir_all(dir.join("config/scat")).unwrap();
    std::fs::write(
        dir.join("config/scat/config.toml"),
        "[cat_mode]\nrender = true\n",
    )
    .unwrap();
    let out = on_pty(&dir, "./cat code.rs", &[]);
    assert!(out.contains('\x1b'), "{out:?}");
    assert!(out.contains("1 │"), "{out:?}");
    // Messages keep the name scat was started under.
    let missing = on_pty(&dir, "./cat missing", &[]);
    assert_eq!(missing, "cat: missing: No such file or directory\n");
}

#[test]
fn scripts_run_from_a_terminal_do_not_page() {
    if !script_available() {
        eprintln!("script not found, skipping");
        return;
    }
    let dir = setup("paging");
    let pager = [("SCAT_PAGER", "/bin/sed s/^/PAGED:/")];
    // Typed at a prompt: scat leads its own process group (here through exec).
    let prompt = on_pty(&dir, &format!("exec '{SCAT}' -p long.txt"), &pager);
    assert!(prompt.contains("PAGED:"), "{prompt:?}");
    // From a script: the shell runs scat inside its own process group.
    let script = on_pty(&dir, &format!("'{SCAT}' -p long.txt; true"), &pager);
    assert!(!script.contains("PAGED:"), "{script:?}");
    assert!(script.contains("line 200"));
    // `always` is an explicit request and still pages there.
    let always = on_pty(
        &dir,
        &format!("'{SCAT}' -p --paging always long.txt; true"),
        &pager,
    );
    assert!(always.contains("PAGED:"), "{always:?}");
}

#[test]
fn terminal_answers_are_cached() {
    if !script_available() {
        eprintln!("script not found, skipping");
        return;
    }
    let dir = setup("cache");
    let runtime = dir.join("run");
    std::fs::create_dir_all(&runtime).unwrap();
    let runtime_str = runtime.to_string_lossy().into_owned();
    let env = [("XDG_RUNTIME_DIR", runtime_str.as_str())];
    let cache = runtime.join("scat");
    let entries = || {
        std::fs::read_dir(&cache)
            .map(|d| d.filter_map(Result::ok).map(|e| e.path()).collect())
            .unwrap_or_else(|_| Vec::new())
    };

    // Plain text and a fixed theme need no query, so nothing is cached.
    on_pty(&dir, &format!("'{SCAT}' --paging never long.txt"), &env);
    on_pty(&dir, &format!("'{SCAT}' --theme Nord code.rs"), &env);
    assert!(entries().is_empty(), "{:?}", entries());

    // Highlighting picks a theme from the background: the query runs (the pty never
    // answers) and the result is cached, private to the user.
    on_pty(&dir, &format!("'{SCAT}' code.rs"), &env);
    let found: Vec<PathBuf> = entries();
    assert_eq!(found.len(), 1, "{found:?}");
    use std::os::unix::fs::PermissionsExt;
    let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&cache), 0o700);
    assert_eq!(mode(&found[0]), 0o600);
    let text = std::fs::read_to_string(&found[0]).unwrap();
    assert!(text.contains("answered=0"), "{text}");
}
