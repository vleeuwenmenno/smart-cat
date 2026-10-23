//! Re-indents minified JSON so the highlighter gets lines instead of one huge one.
//!
//! The text is validated with serde_json but never re-serialised: numbers, string
//! escapes, key order and duplicate keys come out exactly as they went in.

use std::io::{self, BufReader, Cursor, Read};

use crate::input::{Input, InputKind};

/// Files larger than this are not reformatted.
pub const MAX_BYTES: u64 = 16 * 1024 * 1024;
/// A line longer than this on average counts as minified.
const LONG_LINE: usize = 200;
/// A single-line file shorter than this is left alone.
const SHORT_FILE: usize = 80;
const HEAD_BYTES: usize = 4096;

/// True for a `.json` file name or a forced `json` language.
pub fn is_json(file_name: Option<&str>, language: Option<&str>) -> bool {
    if let Some(language) = language {
        return language.eq_ignore_ascii_case("json");
    }
    file_name
        .and_then(|name| name.rsplit_once('.'))
        .is_some_and(|(stem, ext)| !stem.is_empty() && ext.eq_ignore_ascii_case("json"))
}

/// Cheap check on the start of the file: an object or array with no newline in the
/// first 4 KiB, or with very long lines.
fn head_looks_minified(head: &[u8]) -> bool {
    let head = &head[..head.len().min(HEAD_BYTES)];
    let first = head.iter().copied().find(|b| !b.is_ascii_whitespace());
    if !matches!(first, Some(b'{' | b'[')) {
        return false;
    }
    let head = head.trim_ascii_end();
    let lines = head.iter().filter(|&&b| b == b'\n').count() + 1;
    lines == 1 || head.len() / lines > LONG_LINE
}

/// The whole-file decision: a single line over 80 bytes, or lines averaging over 200.
fn is_minified(data: &[u8]) -> bool {
    let trimmed = data.trim_ascii_end();
    let lines = trimmed.iter().filter(|&&b| b == b'\n').count() + 1;
    (lines == 1 && trimmed.len() > SHORT_FILE) || trimmed.len() / lines > LONG_LINE
}

/// Swaps `input`'s reader for the pretty-printed text when the file is valid, minified
/// JSON under [`MAX_BYTES`]. Anything else is left exactly as it was.
pub fn prettify_input(input: &mut Input, head: &[u8]) -> io::Result<()> {
    if !head_looks_minified(head) {
        return Ok(());
    }
    if matches!(input.kind, InputKind::Regular { size } if size > MAX_BYTES) {
        return Ok(());
    }
    let mut data = Vec::new();
    (&mut input.reader)
        .take(MAX_BYTES + 1)
        .read_to_end(&mut data)?;
    if data.len() as u64 <= MAX_BYTES
        && is_minified(&data)
        && let Some(pretty) = pretty(&data)
    {
        input.reader = BufReader::new(Box::new(Cursor::new(pretty)));
        return Ok(());
    }
    // Put back what was read.
    let rest = std::mem::replace(&mut input.reader, BufReader::new(Box::new(io::empty())));
    input.reader = BufReader::new(Box::new(Cursor::new(data).chain(rest)));
    Ok(())
}

/// Two-space indented copy of valid JSON, or `None` when `data` is not JSON.
pub fn pretty(data: &[u8]) -> Option<Vec<u8>> {
    serde_json::from_slice::<serde::de::IgnoredAny>(data).ok()?;
    let mut out = Vec::with_capacity(data.len() * 2);
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    // An opener or comma was written and its line break is still owed.
    let mut owed = false;
    let newline = |out: &mut Vec<u8>, depth: usize| {
        out.push(b'\n');
        out.resize(out.len() + depth * 2, b' ');
    };
    for &b in data {
        if in_string {
            out.push(b);
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == b'"' {
                in_string = false;
            }
            continue;
        }
        match b {
            b' ' | b'\t' | b'\n' | b'\r' => {}
            b'}' | b']' => {
                depth = depth.saturating_sub(1);
                if owed {
                    // Empty container: the opener is still the last byte written.
                    owed = false;
                } else {
                    newline(&mut out, depth);
                }
                out.push(b);
            }
            b',' => {
                out.push(b);
                owed = true;
            }
            b':' => out.extend_from_slice(b": "),
            _ => {
                if owed {
                    newline(&mut out, depth);
                    owed = false;
                }
                out.push(b);
                match b {
                    b'"' => in_string = true,
                    b'{' | b'[' => {
                        depth += 1;
                        owed = true;
                    }
                    _ => {}
                }
            }
        }
    }
    out.push(b'\n');
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(text: &str) -> String {
        String::from_utf8(pretty(text.as_bytes()).unwrap()).unwrap()
    }

    #[test]
    fn indents_by_two_and_keeps_empty_containers() {
        assert_eq!(
            p(r#"{"a":[1,2,{"b":null}],"c":{},"d":[]}"#),
            "{\n  \"a\": [\n    1,\n    2,\n    {\n      \"b\": null\n    }\n  ],\n  \"c\": {},\n  \"d\": []\n}\n"
        );
        assert_eq!(p(" [ ] "), "[]\n");
    }

    #[test]
    fn keeps_order_numbers_and_escapes_exactly() {
        let src = r#"{"z":1.0,"a":1e3,"big":12345678901234567890,"neg":-0.0,"u":"\u00e9\n\"x\\","dup":1,"dup":2,"s":"a,b:{c}[d]"}"#;
        let got = p(src);
        for needle in [
            "\"z\": 1.0,",
            "\"a\": 1e3,",
            "\"big\": 12345678901234567890,",
            "\"neg\": -0.0,",
            r#""u": "\u00e9\n\"x\\","#,
            "\"dup\": 1,\n  \"dup\": 2,",
            r#""s": "a,b:{c}[d]""#,
        ] {
            assert!(got.contains(needle), "{needle} in {got}");
        }
        assert!(got.find("\"z\"").unwrap() < got.find("\"a\"").unwrap());
        // Nothing but whitespace was added.
        let squeezed: String = got.chars().filter(|c| !c.is_ascii_whitespace()).collect();
        assert_eq!(squeezed, src);
    }

    #[test]
    fn invalid_json_is_refused() {
        assert!(pretty(b"{\"a\":}").is_none());
        assert!(pretty(b"{\"a\":1").is_none());
        assert!(pretty(b"not json").is_none());
    }

    #[test]
    fn minified_heuristic() {
        let long = format!("{{\"k\":\"{}\"}}", "x".repeat(100));
        assert!(head_looks_minified(long.as_bytes()));
        assert!(is_minified(long.as_bytes()));
        assert!(is_minified(format!("{long}\n").as_bytes()));
        // Short one-liners stay as they are.
        assert!(!is_minified(b"{\"a\":1}\n"));
        // Ordinary pretty JSON.
        let pretty_json = "{\n  \"a\": 1,\n  \"b\": [\n    2\n  ]\n}\n";
        assert!(!head_looks_minified(pretty_json.as_bytes()));
        assert!(!is_minified(pretty_json.as_bytes()));
        // Long lines on average.
        let wide = format!("[\n\"{}\"\n]\n", "y".repeat(800));
        assert!(head_looks_minified(wide.as_bytes()));
        assert!(is_minified(wide.as_bytes()));
        // Not an object or array.
        assert!(!head_looks_minified(b"\"just a string\""));
    }

    #[test]
    fn detects_json_files() {
        assert!(is_json(Some("a.JSON"), None));
        assert!(is_json(Some("noext"), Some("json")));
        assert!(!is_json(Some("a.jsonl"), None));
        assert!(!is_json(Some(".json"), None));
    }
}
