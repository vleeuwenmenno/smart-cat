//! `--completions SHELL` and `--man`: scripts and the man page, generated from the
//! command line definition so they never drift from the flags.

use std::io::{self, Write};

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
