//! `--completions SHELL` and `--man`: scripts and the man page, generated from the
//! command line definition so they never drift from the flags.

use std::io::{self, Write};
use std::path::{Path, PathBuf};

use clap::CommandFactory;
use clap_complete::Shell;

use crate::cli::Cli;

pub fn completions(shell: Shell) -> i32 {
    finish(write_completions(shell, &mut io::stdout().lock()))
}

pub fn man() -> i32 {
    finish(write_man(&mut io::stdout().lock()))
}

fn finish(result: io::Result<()>) -> i32 {
    match result {
        Ok(()) => 0,
        Err(err) => {
            eprintln!("scat: write error: {}", crate::util::io_message(&err));
            1
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Write files for every shell found on PATH, creating directories as needed.
    Install,
    /// Rewrite only the files that already exist.
    Refresh,
    /// Remove the files.
    Uninstall,
}

/// One file scat can install, and the shell it belongs to (`None` for the man page).
struct Target {
    shell: Option<Shell>,
    path: PathBuf,
}

fn targets() -> Vec<Target> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    let xdg = |var: &str, fallback: &str| {
        std::env::var_os(var)
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .unwrap_or_else(|| home.join(fallback))
    };
    let config = xdg("XDG_CONFIG_HOME", ".config");
    let data = xdg("XDG_DATA_HOME", ".local/share");
    vec![
        Target {
            shell: Some(Shell::Fish),
            path: config.join("fish/completions/scat.fish"),
        },
        Target {
            shell: Some(Shell::Bash),
            path: data.join("bash-completion/completions/scat"),
        },
        Target {
            shell: Some(Shell::Zsh),
            path: data.join("zsh/site-functions/_scat"),
        },
        Target {
            shell: None,
            path: data.join("man/man1/scat.1"),
        },
    ]
}

/// `--install-completions`, `--uninstall-completions` and the hidden
/// `--refresh-completions` run after `--update`.
pub fn install(mode: Mode) -> i32 {
    if std::env::var_os("HOME").is_none() {
        eprintln!("scat: HOME is not set");
        return 1;
    }
    let mut status = 0;
    let mut zsh_written = None;
    for target in targets() {
        let shown = crate::render::sanitize_str(&target.path.to_string_lossy());
        let label = target
            .shell
            .map_or("man page".to_string(), |s| format!("{s} completions"));
        let wanted = match mode {
            Mode::Install => target
                .shell
                .is_none_or(|shell| crate::util::which(&shell.to_string()).is_some()),
            Mode::Refresh | Mode::Uninstall => target.path.exists(),
        };
        if !wanted {
            continue;
        }
        let result = match mode {
            Mode::Uninstall => std::fs::remove_file(&target.path),
            Mode::Install | Mode::Refresh => write_target(&target),
        };
        match result {
            Ok(()) if mode == Mode::Uninstall => println!("removed {label}: {shown}"),
            Ok(()) => {
                if mode == Mode::Install {
                    println!("installed {label}: {shown}");
                }
                if target.shell == Some(Shell::Zsh) {
                    zsh_written = target.path.parent().map(Path::to_path_buf);
                }
            }
            Err(err) => {
                eprintln!("scat: {shown}: {}", crate::util::io_message(&err));
                status = 1;
            }
        }
    }
    if mode == Mode::Install
        && let Some(dir) = zsh_written
    {
        println!(
            "zsh: if completion does not work, add this to ~/.zshrc before compinit:\n  fpath=({} $fpath)",
            crate::render::sanitize_str(&dir.to_string_lossy())
        );
    }
    status
}

/// Writes next to the target and renames, so a half-written file is never loaded.
fn write_target(target: &Target) -> io::Result<()> {
    let mut content = Vec::new();
    match target.shell {
        Some(shell) => write_completions(shell, &mut content)?,
        None => write_man(&mut content)?,
    }
    let dir = target
        .path
        .parent()
        .ok_or_else(|| io::Error::other("no parent directory"))?;
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join(format!(".scat-{}.tmp", std::process::id()));
    std::fs::write(&tmp, &content)?;
    std::fs::rename(&tmp, &target.path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

fn write_completions<W: Write>(shell: Shell, out: &mut W) -> io::Result<()> {
    let mut command = Cli::command();
    let mut buf = Vec::new();
    clap_complete::generate(shell, &mut command, "scat", &mut buf);
    out.write_all(&buf)
}

fn write_man<W: Write>(out: &mut W) -> io::Result<()> {
    clap_mangen::Man::new(Cli::command()).render(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completions_mention_flags_for_each_shell() {
        for shell in [Shell::Fish, Shell::Bash, Shell::Zsh] {
            let mut out = Vec::new();
            write_completions(shell, &mut out).unwrap();
            let text = String::from_utf8(out).unwrap();
            assert!(text.contains("line-numbers"), "{shell}");
            assert!(text.contains("completions"), "{shell}");
        }
    }

    #[test]
    fn man_page_is_roff() {
        let mut out = Vec::new();
        write_man(&mut out).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains(".TH"));
        assert!(text.contains("scat"));
    }
}
