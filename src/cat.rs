//! GNU cat compatible output: line numbering, squeezing and ^/M- notation.

use std::io::{self, BufReader, Read, Write};

use crate::cli::Cli;
use crate::util::read_line_capped;

const MAX_LINE: usize = 64 * 1024;

#[derive(Debug, Clone, Copy, Default)]
pub struct CatOptions {
    pub number: bool,
    pub number_nonblank: bool,
    pub squeeze_blank: bool,
    pub show_ends: bool,
    pub show_tabs: bool,
    pub show_nonprinting: bool,
}

impl CatOptions {
    pub fn from_cli(cli: &Cli) -> Self {
        Self {
            number: cli.number,
            number_nonblank: cli.number_nonblank,
            squeeze_blank: cli.squeeze_blank,
            show_ends: cli.show_ends || cli.show_all || cli.show_ends_nonprinting,
            show_tabs: cli.show_tabs || cli.show_all || cli.show_tabs_nonprinting,
            show_nonprinting: cli.show_nonprinting
                || cli.show_all
                || cli.show_ends_nonprinting
                || cli.show_tabs_nonprinting,
        }
    }

    /// True when output is a plain byte copy of the input.
    pub fn is_passthrough(&self) -> bool {
        !(self.number
            || self.number_nonblank
            || self.squeeze_blank
            || self.show_ends
            || self.show_tabs
            || self.show_nonprinting)
    }
}

/// State carried across files, as GNU cat does: numbering continues, and a file that
/// does not end in a newline continues on the next file's first line.
#[derive(Debug)]
pub struct CatState {
    line: u64,
    at_line_start: bool,
    blank_run: u32,
}

impl Default for CatState {
    fn default() -> Self {
        Self {
            line: 0,
            at_line_start: true,
            blank_run: 0,
        }
    }
}

/// Copies `reader` to `out` unchanged, flushing whenever the input would block so
/// streamed input (`tail -f log | scat`) shows up immediately.
pub fn copy_raw<R: Read, W: Write>(reader: &mut R, out: &mut W) -> io::Result<()> {
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = match reader.read(&mut buf) {
            Ok(0) => return Ok(()),
            Ok(n) => n,
            Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
            Err(err) => return Err(err),
        };
        out.write_all(&buf[..n])?;
        out.flush()?;
    }
}

pub fn process<R: Read, W: Write>(
    reader: &mut BufReader<R>,
    out: &mut W,
    opts: &CatOptions,
    state: &mut CatState,
) -> io::Result<()> {
    let mut line = Vec::with_capacity(8 * 1024);
    loop {
        if reader.buffer().is_empty() {
            out.flush()?;
        }
        let Some(terminated) = read_line_capped(reader, &mut line, MAX_LINE)? else {
            return Ok(());
        };
        let body = if terminated {
            &line[..line.len() - 1]
        } else {
            &line[..]
        };

        if state.at_line_start {
            let blank = terminated && body.is_empty();
            if blank {
                if opts.squeeze_blank && state.blank_run > 0 {
                    continue;
                }
                state.blank_run += 1;
            } else {
                state.blank_run = 0;
            }
            if opts.number_nonblank {
                if !blank {
                    write_number(out, state)?;
                }
            } else if opts.number {
                write_number(out, state)?;
            }
        }

        // Like coreutils 9, -E marks a carriage return before the newline as ^M.
        let cr_before_newline =
            terminated && opts.show_ends && !opts.show_nonprinting && body.last() == Some(&b'\r');
        if cr_before_newline {
            write_body(out, &body[..body.len() - 1], opts)?;
            out.write_all(b"^M")?;
        } else {
            write_body(out, body, opts)?;
        }
        if terminated {
            if opts.show_ends {
                out.write_all(b"$")?;
            }
            out.write_all(b"\n")?;
        }
        state.at_line_start = terminated;
    }
}

fn write_number<W: Write>(out: &mut W, state: &mut CatState) -> io::Result<()> {
    state.line += 1;
    write!(out, "{:>6}\t", state.line)
}

fn write_body<W: Write>(out: &mut W, body: &[u8], opts: &CatOptions) -> io::Result<()> {
    if !opts.show_nonprinting && !opts.show_tabs {
        return out.write_all(body);
    }
    let mut encoded = Vec::with_capacity(body.len() + 16);
    for &byte in body {
        if byte == b'\t' {
            if opts.show_tabs {
                encoded.extend_from_slice(b"^I");
            } else {
                encoded.push(b'\t');
            }
        } else if opts.show_nonprinting {
            push_caret_notation(&mut encoded, byte);
        } else {
            encoded.push(byte);
        }
    }
    out.write_all(&encoded)
}

/// cat -v notation: control characters as ^X, DEL as ^?, high bytes as M-x.
pub fn push_caret_notation(out: &mut Vec<u8>, byte: u8) {
    let low = if byte >= 128 {
        out.extend_from_slice(b"M-");
        byte - 128
    } else {
        byte
    };
    match low {
        0..=31 => {
            out.push(b'^');
            out.push(low + 64);
        }
        127 => out.extend_from_slice(b"^?"),
        _ => out.push(low),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(input: &[u8], opts: CatOptions) -> String {
        let mut out = Vec::new();
        let mut state = CatState::default();
        process(
            &mut BufReader::new(io::Cursor::new(input)),
            &mut out,
            &opts,
            &mut state,
        )
        .unwrap();
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn numbers_lines() {
        let opts = CatOptions {
            number: true,
            ..Default::default()
        };
        assert_eq!(run(b"a\n\nb", opts), "     1\ta\n     2\t\n     3\tb");
    }

    #[test]
    fn numbers_nonblank_and_squeezes() {
        let opts = CatOptions {
            number_nonblank: true,
            squeeze_blank: true,
            ..Default::default()
        };
        assert_eq!(run(b"a\n\n\n\nb\n", opts), "     1\ta\n\n     2\tb\n");
    }

    #[test]
    fn shows_nonprinting() {
        let opts = CatOptions {
            show_nonprinting: true,
            show_ends: true,
            show_tabs: true,
            ..Default::default()
        };
        assert_eq!(run(b"\x1b[0m\t\x7f\xe9\n", opts), "^[[0m^I^?M-i$\n");
    }

    #[test]
    fn numbering_continues_across_files() {
        let opts = CatOptions {
            number: true,
            ..Default::default()
        };
        let mut out = Vec::new();
        let mut state = CatState::default();
        process(
            &mut BufReader::new(io::Cursor::new(b"a\nb")),
            &mut out,
            &opts,
            &mut state,
        )
        .unwrap();
        process(
            &mut BufReader::new(io::Cursor::new(b"c\nd\n")),
            &mut out,
            &opts,
            &mut state,
        )
        .unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "     1\ta\n     2\tbc\n     3\td\n"
        );
    }
}
