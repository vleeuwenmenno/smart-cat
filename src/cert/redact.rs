//! Hides private key bodies in text that the `auto` and `cert` renderers print instead
//! of the certificate view: files over the size limit, files that are not UTF-8,
//! blocks that do not parse, and the text and hex fallbacks.
//!
//! The scanner is line based and works on bytes. A `BEGIN` marker whose label contains
//! `PRIVATE KEY` (any case, extra whitespace, anywhere in the line) starts hiding: the
//! rest of that line and every following line is replaced by one note, until the
//! matching `END` marker or the end of the input. Markers of other blocks nested inside
//! a hidden block are hidden too; a nested private key keeps the outer one open until
//! its own `END`.

use std::io::{self, BufRead, Read};

use crate::util::read_line_capped;

/// Lines longer than this are scanned in pieces of this size, so memory stays bounded.
/// A marker split across two pieces of such a line is not seen.
pub const MAX_LINE: usize = 64 * 1024;

/// One `-----BEGIN X-----` or `-----END X-----` marker found in a line.
struct Marker {
    begin: bool,
    /// Upper case, `-` and `_` as spaces, whitespace collapsed.
    label: String,
    /// Byte range of the whole marker in the line.
    start: usize,
    end: usize,
}

/// Finds the PEM markers in a line: at least three dashes, optional spaces, `BEGIN` or
/// `END` (any case), a label, and the dashes after it (or the end of the line).
fn markers(line: &[u8]) -> Vec<Marker> {
    let mut found = Vec::new();
    // Quick exit: almost every line has no marker.
    if !line.windows(3).any(|w| w == b"---") {
        return found;
    }
    let upper = line.to_ascii_uppercase();
    let mut i = 0;
    while i + 3 <= upper.len() {
        if &upper[i..i + 3] != b"---" {
            i += 1;
            continue;
        }
        let start = i;
        let mut j = i;
        while j < upper.len() && upper[j] == b'-' {
            j += 1;
        }
        while j < upper.len() && upper[j].is_ascii_whitespace() {
            j += 1;
        }
        let begin = if upper[j..].starts_with(b"BEGIN") {
            j += 5;
            true
        } else if upper[j..].starts_with(b"END") {
            j += 3;
            false
        } else {
            i = j.max(i + 1);
            continue;
        };
        let label_start = j;
        let label_end = upper[j..]
            .windows(3)
            .position(|w| w == b"---")
            .map_or(upper.len(), |p| j + p);
        let mut end = label_end;
        while end < upper.len() && upper[end] == b'-' {
            end += 1;
        }
        let label = String::from_utf8_lossy(&upper[label_start..label_end])
            .replace(['-', '_'], " ")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        found.push(Marker {
            begin,
            label,
            start,
            end,
        });
        i = end.max(i + 1);
    }
    found
}

fn is_private(label: &str) -> bool {
    label.contains("PRIVATE KEY")
}

/// Line-by-line redaction state.
#[derive(Default)]
pub struct Scanner {
    /// Labels of the private key blocks that are open, outermost first.
    open: Vec<String>,
}

impl Scanner {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether the scanner is inside a private key block.
    pub fn hiding(&self) -> bool {
        !self.open.is_empty()
    }

    /// Appends what to show for `line` (one line or piece of a line, with or without
    /// its terminator) to `out`. Hidden lines add nothing; the line that starts hiding
    /// is cut after its `BEGIN` marker and followed by a line with `note`.
    pub fn push(&mut self, line: &[u8], note: &str, out: &mut Vec<u8>) {
        let found = markers(line);
        if found.is_empty() {
            if !self.hiding() {
                out.extend_from_slice(line);
            }
            return;
        }
        // Start of the part of the line that is still to be shown.
        let mut shown_from = if self.hiding() { None } else { Some(0) };
        for marker in found {
            if self.hiding() {
                if marker.begin {
                    if is_private(&marker.label) {
                        self.open.push(marker.label);
                    }
                } else if let Some(idx) = self.open.iter().position(|l| *l == marker.label) {
                    self.open.truncate(idx);
                    if self.open.is_empty() {
                        shown_from = Some(marker.start);
                    }
                }
            } else if marker.begin && is_private(&marker.label) {
                let from = shown_from.take().unwrap_or(0);
                out.extend_from_slice(&line[from..marker.end]);
                out.push(b'\n');
                out.extend_from_slice(note.as_bytes());
                out.push(b'\n');
                self.open.push(marker.label);
            }
        }
        if let Some(from) = shown_from {
            out.extend_from_slice(&line[from..]);
        }
    }
}

/// A reader that passes its input through a [`Scanner`].
pub struct Redacting<R> {
    inner: R,
    scanner: Scanner,
    note: String,
    line: Vec<u8>,
    out: Vec<u8>,
    pos: usize,
    done: bool,
}

impl<R: BufRead> Redacting<R> {
    pub fn new(inner: R) -> Self {
        Self {
            inner,
            scanner: Scanner::new(),
            note: format!("({})", super::HIDDEN_NOTE),
            line: Vec::new(),
            out: Vec::new(),
            pos: 0,
            done: false,
        }
    }
}

impl<R: BufRead> Read for Redacting<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        while self.pos == self.out.len() {
            if self.done {
                return Ok(0);
            }
            self.out.clear();
            self.pos = 0;
            if read_line_capped(&mut self.inner, &mut self.line, MAX_LINE)?.is_none() {
                self.done = true;
                return Ok(0);
            }
            self.scanner.push(&self.line, &self.note, &mut self.out);
        }
        let n = buf.len().min(self.out.len() - self.pos);
        buf[..n].copy_from_slice(&self.out[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn redact(text: &str) -> String {
        let mut out = String::new();
        Redacting::new(text.as_bytes())
            .read_to_string(&mut out)
            .unwrap();
        out
    }

    #[test]
    fn hides_body_until_end() {
        let out = redact("a\n-----BEGIN PRIVATE KEY-----\nSECRET\n-----END PRIVATE KEY-----\nb\n");
        assert!(!out.contains("SECRET"), "{out}");
        assert!(
            out.starts_with("a\n-----BEGIN PRIVATE KEY-----\n("),
            "{out}"
        );
        assert!(out.ends_with("-----END PRIVATE KEY-----\nb\n"), "{out}");
    }

    #[test]
    fn leaves_other_blocks_alone() {
        let text = "-----BEGIN CERTIFICATE-----\nMIIB\n-----END CERTIFICATE-----\n";
        assert_eq!(redact(text), text);
    }

    #[test]
    fn variants_are_hidden() {
        for begin in [
            "-----begin private key-----",
            "  -----BEGIN   ENCRYPTED  PRIVATE KEY-----  ",
            "----- BEGIN OPENSSH PRIVATE KEY -----\r",
            "key: \"-----BEGIN RSA PRIVATE KEY-----",
            "-----BEGIN PRIVATE KEY",
            "---BEGIN PRIVATE-KEY---",
        ] {
            let out = redact(&format!("{begin}\nSECRET\n"));
            assert!(!out.contains("SECRET"), "{begin}: {out}");
        }
    }

    #[test]
    fn one_line_block_is_cut() {
        let out = redact("x -----BEGIN PRIVATE KEY-----SECRET-----END PRIVATE KEY----- y\nz\n");
        assert!(!out.contains("SECRET"), "{out}");
        assert!(out.contains("-----END PRIVATE KEY----- y\nz\n"), "{out}");
    }

    #[test]
    fn nested_private_key_keeps_outer_open() {
        let out = redact(
            "-----BEGIN PRIVATE KEY-----\nAAA\n-----BEGIN EC PRIVATE KEY-----\nBBB\n\
             -----END EC PRIVATE KEY-----\nCCC\n-----END PRIVATE KEY-----\nok\n",
        );
        for secret in ["AAA", "BBB", "CCC"] {
            assert!(!out.contains(secret), "{out}");
        }
        assert!(out.ends_with("ok\n"));
    }

    #[test]
    fn mismatched_end_hides_to_eof() {
        let out = redact("-----BEGIN RSA PRIVATE KEY-----\nAAA\n-----END PRIVATE KEY-----\nBBB\n");
        assert!(!out.contains("AAA") && !out.contains("BBB"), "{out}");
    }

    #[test]
    fn long_lines_are_bounded() {
        let long = "x".repeat(3 * MAX_LINE);
        let out = redact(&format!("{long}\n-----BEGIN PRIVATE KEY-----\n{long}\n"));
        assert!(out.starts_with(&long));
        assert!(!out.contains(&format!("\n{}", &long[..10])));
    }
}
