//! Hex dump view for binary files: offset gutter, hex bytes, ASCII pane.
//!
//! Only fixed glyphs and hex digits are written, so nothing from the file can reach the
//! terminal as an escape sequence.

use std::fmt::Write as _;
use std::io::{self, Read, Write};

use crate::util::human_size;

pub const DEFAULT_MAX_BYTES: usize = 512;
const PER_LINE: usize = 16;

pub struct Options {
    pub colors: bool,
    pub utf8: bool,
    /// How many leading bytes to show.
    pub max_bytes: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Class {
    Null,
    Printable,
    Whitespace,
    Other,
}

fn classify(byte: u8) -> Class {
    match byte {
        0 => Class::Null,
        b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c => Class::Whitespace,
        0x21..=0x7e => Class::Printable,
        _ => Class::Other,
    }
}

fn sgr(class: Class) -> &'static str {
    match class {
        Class::Null => "\x1b[2m",
        Class::Printable => "\x1b[36m",
        Class::Whitespace => "\x1b[32m",
        Class::Other => "\x1b[33m",
    }
}

/// Shows the first `max_bytes` of `reader`. `total` is the file size when known.
pub fn render<R: Read, W: Write>(
    out: &mut W,
    reader: &mut R,
    total: Option<u64>,
    opts: &Options,
) -> io::Result<()> {
    let mut data = Vec::with_capacity(opts.max_bytes.min(1 << 20));
    reader
        .by_ref()
        .take(opts.max_bytes as u64)
        .read_to_end(&mut data)?;
    // One more byte tells whether the dump is truncated, also for streams of unknown size.
    let mut probe = [0u8; 1];
    let truncated = reader.read(&mut probe)? > 0;
    out.write_all(&format(&data, total, truncated, opts))
}

/// The dump as bytes. `truncated` says more data follows `data`.
pub fn format(data: &[u8], total: Option<u64>, truncated: bool, opts: &Options) -> Vec<u8> {
    let (dim, undim, reset) = if opts.colors {
        ("\x1b[2m", "\x1b[22m", "\x1b[0m")
    } else {
        ("", "", "")
    };
    let (bar, dot) = if opts.utf8 { ("│", '·') } else { ("|", '.') };
    let mut text = String::new();
    for (index, chunk) in data.chunks(PER_LINE).enumerate() {
        let _ = write!(text, "{dim}{:08x}{undim}  ", index * PER_LINE);
        for slot in 0..PER_LINE {
            if slot == 8 {
                text.push(' ');
            }
            match chunk.get(slot) {
                Some(&byte) if opts.colors => {
                    let _ = write!(text, "{}{byte:02x}{reset} ", sgr(classify(byte)));
                }
                Some(&byte) => {
                    let _ = write!(text, "{byte:02x} ");
                }
                None => text.push_str("   "),
            }
        }
        let _ = write!(text, " {dim}{bar}{undim}");
        for &byte in chunk {
            let class = classify(byte);
            let shown = match class {
                Class::Printable => char::from(byte),
                Class::Whitespace if byte == b' ' => ' ',
                _ => dot,
            };
            if opts.colors {
                let _ = write!(text, "{}{shown}{reset}", sgr(class));
            } else {
                text.push(shown);
            }
        }
        let _ = writeln!(text, "{dim}{bar}{undim}");
    }
    if truncated {
        let more = match total {
            Some(total) if total > data.len() as u64 => {
                format!("{} more", human_size(total - data.len() as u64))
            }
            _ => "more".to_string(),
        };
        let ellipsis = if opts.utf8 { "…" } else { "..." };
        let _ = writeln!(text, "{dim}{ellipsis} {more}, use --raw for bytes{undim}");
    }
    text.into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(utf8: bool) -> Options {
        Options {
            colors: false,
            utf8,
            max_bytes: 512,
        }
    }

    #[test]
    fn formats_a_full_and_a_partial_line() {
        let data = b"\x7fELF\x02\x01\x01\0 hello world\n";
        let out = String::from_utf8(format(data, None, false, &plain(true))).unwrap();
        let expected = [
            "00000000  7f 45 4c 46 02 01 01 00  20 68 65 6c 6c 6f 20 77  │·ELF···· hello w│",
            "00000010  6f 72 6c 64 0a                                    │orld·│",
            "",
        ]
        .join("\n");
        assert_eq!(out, expected);
    }

    #[test]
    fn ascii_fallback_and_truncation_note() {
        let out = String::from_utf8(format(
            b"ab\0",
            Some(3 + 12 * 1024 * 1024 + 300_000),
            true,
            &plain(false),
        ))
        .unwrap();
        let expected = [
            "00000000  61 62 00                                          |ab.|",
            "... 12.3 MiB more, use --raw for bytes",
            "",
        ]
        .join("\n");
        assert_eq!(out, expected);
    }

    #[test]
    fn colours_by_byte_class() {
        let opts = Options {
            colors: true,
            ..plain(true)
        };
        let out = String::from_utf8(format(b"\0A \xff", None, false, &opts)).unwrap();
        assert!(out.contains("\x1b[2m00\x1b[0m "));
        assert!(out.contains("\x1b[36m41\x1b[0m "));
        assert!(out.contains("\x1b[32m20\x1b[0m "));
        assert!(out.contains("\x1b[33mff\x1b[0m "));
    }

    #[test]
    fn render_reads_only_max_bytes_and_detects_truncation() {
        let data = [b'x'; 40];
        let mut out = Vec::new();
        let opts = Options {
            max_bytes: 16,
            ..plain(false)
        };
        render(&mut out, &mut &data[..], Some(40), &opts).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert_eq!(text.lines().count(), 2);
        assert!(text.ends_with("... 24 B more, use --raw for bytes\n"));
    }
}
