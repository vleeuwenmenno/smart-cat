//! Line-by-line colouring for diffs and logs. Both work on one line at a time, so they
//! stream: `tail -f app.log | scat -l log` shows each line as it arrives.

use std::ops::Range;

pub const BOLD: &str = "\x1b[1m";
pub const DIM: &str = "\x1b[2m";
const RED: &str = "\x1b[31m";
const BOLD_RED: &str = "\x1b[1;31m";
const GREEN: &str = "\x1b[32m";
const YELLOW: &str = "\x1b[33m";
const CYAN: &str = "\x1b[36m";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Diff,
    Log,
}

/// A piece of a line and the SGR sequence to draw it with (empty for plain).
pub type Span = (Range<usize>, &'static str);

#[derive(Debug)]
pub struct Styler {
    kind: Kind,
    diff: DiffState,
}

impl Styler {
    pub fn new(kind: Kind) -> Self {
        Self {
            kind,
            diff: DiffState::default(),
        }
    }

    /// Splits one line (without its line ending) into styled spans covering all of it.
    pub fn spans(&mut self, line: &str) -> Vec<Span> {
        match self.kind {
            Kind::Diff => vec![(0..line.len(), self.diff.style(line))],
            Kind::Log => log_spans(line),
        }
    }
}

// ---- diff ---------------------------------------------------------------------------

#[derive(Debug, Default)]
struct DiffState {
    in_hunk: bool,
    /// Whether the hunk header gave line counts (a combined diff, `@@@`, does not).
    counted: bool,
    old_left: u64,
    new_left: u64,
}

impl DiffState {
    fn style(&mut self, line: &str) -> &'static str {
        let first = line.as_bytes().first().copied();
        if self.in_hunk && self.counted {
            let style = match first {
                Some(b'+') if self.new_left > 0 => {
                    self.new_left -= 1;
                    Some(GREEN)
                }
                Some(b'-') if self.old_left > 0 => {
                    self.old_left -= 1;
                    Some(RED)
                }
                Some(b' ') | None if self.old_left > 0 && self.new_left > 0 => {
                    self.old_left -= 1;
                    self.new_left -= 1;
                    Some("")
                }
                Some(b'\\') => Some(DIM),
                _ => None,
            };
            if self.old_left == 0 && self.new_left == 0 {
                self.in_hunk = false;
            }
            if let Some(style) = style {
                return style;
            }
            self.in_hunk = false;
        }
        if line.starts_with("@@") {
            self.start_hunk(line);
            return CYAN;
        }
        if self.in_hunk && !line.starts_with("diff ") {
            match first {
                Some(b'+') => return GREEN,
                Some(b'-') => return RED,
                Some(b'\\') => return DIM,
                _ => return "",
            }
        }
        self.in_hunk = false;
        const HEADERS: [&str; 15] = [
            "diff ",
            "index ",
            "--- ",
            "+++ ",
            "new file mode",
            "deleted file mode",
            "old mode",
            "new mode",
            "similarity index",
            "dissimilarity index",
            "rename from",
            "rename to",
            "copy from",
            "Binary files",
            "Index: ",
        ];
        if HEADERS.iter().any(|h| line.starts_with(h)) || line.starts_with("====") {
            BOLD
        } else {
            ""
        }
    }

    /// Reads `@@ -a,b +c,d @@`; anything else (`@@@` combined diffs) is an uncounted hunk.
    fn start_hunk(&mut self, line: &str) {
        self.in_hunk = true;
        self.counted = false;
        let Some(rest) = line.strip_prefix("@@ -") else {
            return;
        };
        let mut parts = rest.split(' ');
        let old = parts.next().and_then(range_len);
        let new = parts
            .next()
            .and_then(|p| p.strip_prefix('+'))
            .and_then(range_len);
        if let (Some(old), Some(new)) = (old, new) {
            self.counted = true;
            self.old_left = old;
            self.new_left = new;
            self.in_hunk = old > 0 || new > 0;
        }
    }
}

/// The line count of `a,b` or `a` (which means 1).
fn range_len(range: &str) -> Option<u64> {
    match range.split_once(',') {
        Some((start, len)) => {
            start.parse::<u64>().ok()?;
            len.parse().ok()
        }
        None => range.parse::<u64>().ok().map(|_| 1),
    }
}

/// True if the first lines of `head` look like a unified diff.
pub fn looks_like_diff(head: &[u8]) -> bool {
    let text = String::from_utf8_lossy(head);
    let lines: Vec<&str> = text.lines().collect();
    if lines.iter().any(|l| l.starts_with("diff --git ")) {
        return true;
    }
    lines
        .windows(3)
        .any(|w| w[0].starts_with("--- ") && w[1].starts_with("+++ ") && w[2].starts_with("@@ -"))
}

// ---- log ----------------------------------------------------------------------------

fn log_spans(line: &str) -> Vec<Span> {
    let bytes = line.as_bytes();
    let mut spans = Vec::new();
    let ts_end = timestamp_len(bytes).unwrap_or(0);
    if ts_end > 0 {
        spans.push((0..ts_end, DIM));
    }
    let json = bytes.first() == Some(&b'{');
    if let Some((range, style)) = find_level(bytes, ts_end, json) {
        if range.start > ts_end {
            spans.push((ts_end..range.start, ""));
        }
        let end = range.end;
        spans.push((range, style));
        if end < line.len() {
            spans.push((end..line.len(), ""));
        }
    } else if ts_end < line.len() {
        spans.push((ts_end..line.len(), ""));
    }
    if spans.is_empty() {
        spans.push((0..0, ""));
    }
    spans
}

fn digits(bytes: &[u8], at: usize, n: usize) -> bool {
    bytes.len() >= at + n && bytes[at..at + n].iter().all(u8::is_ascii_digit)
}

/// Length of a leading timestamp (with its brackets): ISO 8601 / `2026-10-23 13:42:10`
/// with optional fraction and zone, syslog `Oct 23 13:42:10`, or a bare `13:42:10.123`.
pub fn timestamp_len(bytes: &[u8]) -> Option<usize> {
    let open = usize::from(bytes.first() == Some(&b'['));
    let mut i = open;
    if digits(bytes, i, 4) && bytes.get(i + 4) == Some(&b'-') && digits(bytes, i + 5, 2) {
        // 2026-10-23, then T or space, then a clock time.
        if bytes.get(i + 7) != Some(&b'-') || !digits(bytes, i + 8, 2) {
            return None;
        }
        i += 10;
        if matches!(bytes.get(i), Some(b'T' | b' ')) && clock_len(bytes, i + 1).is_some() {
            i += 1 + clock_len(bytes, i + 1)?;
            i += fraction_len(bytes, i);
            i += zone_len(bytes, i);
        }
    } else if let Some(month) = bytes.get(i..i + 3)
        && is_month(month)
        && bytes.get(i + 3) == Some(&b' ')
    {
        // Oct 23 13:42:10, the day space-padded for 1 to 9.
        let day = i + 4;
        if !(bytes
            .get(day)
            .is_some_and(|b| *b == b' ' || b.is_ascii_digit())
            && bytes.get(day + 1).is_some_and(u8::is_ascii_digit)
            && bytes.get(day + 2) == Some(&b' '))
        {
            return None;
        }
        i = day + 3 + clock_len(bytes, day + 3)?;
    } else {
        i += clock_len(bytes, i)?;
        i += fraction_len(bytes, i);
        if open == 0 && bytes.get(i).is_some_and(|b| b.is_ascii_alphanumeric()) {
            return None;
        }
    }
    if open == 1 {
        if bytes.get(i) != Some(&b']') {
            return None;
        }
        i += 1;
    }
    Some(i)
}

fn is_month(m: &[u8]) -> bool {
    const MONTHS: [&[u8]; 12] = [
        b"Jan", b"Feb", b"Mar", b"Apr", b"May", b"Jun", b"Jul", b"Aug", b"Sep", b"Oct", b"Nov",
        b"Dec",
    ];
    MONTHS.contains(&m)
}

/// `HH:MM:SS` at `at`.
fn clock_len(bytes: &[u8], at: usize) -> Option<usize> {
    (digits(bytes, at, 2)
        && bytes.get(at + 2) == Some(&b':')
        && digits(bytes, at + 3, 2)
        && bytes.get(at + 5) == Some(&b':')
        && digits(bytes, at + 6, 2))
    .then_some(8)
}

fn fraction_len(bytes: &[u8], at: usize) -> usize {
    if matches!(bytes.get(at), Some(b'.' | b',')) && digits(bytes, at + 1, 1) {
        1 + bytes[at + 1..]
            .iter()
            .take_while(|b| b.is_ascii_digit())
            .count()
    } else {
        0
    }
}

fn zone_len(bytes: &[u8], at: usize) -> usize {
    match bytes.get(at) {
        Some(b'Z') => 1,
        Some(b'+' | b'-') if digits(bytes, at + 1, 2) => {
            if bytes.get(at + 3) == Some(&b':') && digits(bytes, at + 4, 2) {
                6
            } else if digits(bytes, at + 3, 2) {
                5
            } else {
                3
            }
        }
        _ => 0,
    }
}

const LEVELS: [(&str, &str); 22] = [
    ("FATAL", BOLD_RED),
    ("CRIT", BOLD_RED),
    ("CRITICAL", BOLD_RED),
    ("PANIC", BOLD_RED),
    ("EMERG", BOLD_RED),
    ("EMERGENCY", BOLD_RED),
    ("ALERT", BOLD_RED),
    ("SEVERE", RED),
    ("ERROR", RED),
    ("ERR", RED),
    ("WARN", YELLOW),
    ("WARNING", YELLOW),
    ("INFO", GREEN),
    ("NOTICE", GREEN),
    ("DEBUG", DIM),
    ("TRACE", DIM),
    ("VERBOSE", DIM),
    ("FINE", DIM),
    ("DBG", DIM),
    ("INF", GREEN),
    ("WRN", YELLOW),
    ("ERRO", RED),
];

fn level_style(word: &str) -> Option<&'static str> {
    LEVELS
        .iter()
        .find(|(name, _)| word.eq_ignore_ascii_case(name))
        .map(|(_, style)| *style)
}

/// The first level keyword: an upper-case word near the start of a plain log line, or
/// any-case value of a `level=` / `"level":` field.
fn find_level(bytes: &[u8], from: usize, json: bool) -> Option<(Range<usize>, &'static str)> {
    let window = bytes.len().min(if json { 400 } else { from + 100 });
    let mut i = from;
    while i < window {
        if !bytes[i].is_ascii_alphabetic() || (i > 0 && is_word_byte(bytes[i - 1])) {
            i += 1;
            continue;
        }
        let end = i + bytes[i..]
            .iter()
            .take_while(|b| b.is_ascii_alphabetic())
            .count();
        if bytes.get(end).is_some_and(|b| is_word_byte(*b)) {
            i = end;
            continue;
        }
        let word = std::str::from_utf8(&bytes[i..end]).ok()?;
        if let Some(style) = level_style(word) {
            let upper = word.bytes().all(|b| b.is_ascii_uppercase());
            if (upper && !json) || after_level_key(&bytes[..i]) {
                return Some((i..end, style));
            }
        }
        i = end;
    }
    None
}

fn is_word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// True if `before` ends with a level key and optional quote or space: `level=`,
/// `"level": "`.
fn after_level_key(before: &[u8]) -> bool {
    let tail: String = before[before.len().saturating_sub(14)..]
        .iter()
        .map(|b| char::from(b.to_ascii_lowercase()))
        .filter(|c| !matches!(c, '"' | ' ' | '\''))
        .collect();
    [
        "level=",
        "lvl=",
        "severity=",
        "level:",
        "levelname=",
        "levelname:",
    ]
    .iter()
    .any(|k| tail.ends_with(k))
}

/// True if the first lines of `head` look like a log: most start with a timestamp.
pub fn looks_like_log(head: &[u8]) -> bool {
    let mut total = 0;
    let mut stamped = 0;
    for line in head.split(|&b| b == b'\n').take(10) {
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        total += 1;
        if timestamp_len(line).is_some() {
            stamped += 1;
        }
    }
    stamped >= 2 && stamped * 10 >= total * 6
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pieces<'a>(line: &'a str, spans: &[Span]) -> Vec<(&'a str, &'static str)> {
        spans
            .iter()
            .map(|(r, s)| (&line[r.clone()], *s))
            .filter(|(t, _)| !t.is_empty())
            .collect()
    }

    fn diff_styles(lines: &[&str]) -> Vec<&'static str> {
        let mut styler = Styler::new(Kind::Diff);
        lines.iter().map(|l| styler.spans(l).remove(0).1).collect()
    }

    #[test]
    fn diff_hunk_counts_decide_headers() {
        let styles = diff_styles(&[
            "diff --git a/f b/f",
            "--- a/f",
            "+++ b/f",
            "@@ -1 +1 @@ fn x",
            "--- removed dashes",
            "+++ added pluses",
            "--- a/g",
            "+++ b/g",
            "@@ -1 +1 @@",
            " ctx",
        ]);
        assert_eq!(
            styles,
            [BOLD, BOLD, BOLD, CYAN, RED, GREEN, BOLD, BOLD, CYAN, ""]
        );
    }

    #[test]
    fn diff_no_newline_marker_is_dim() {
        let styles = diff_styles(&["@@ -1 +1 @@", "-a", "\\ No newline at end of file", "+b"]);
        assert_eq!(styles, [CYAN, RED, DIM, GREEN]);
    }

    #[test]
    fn sniffs_diffs() {
        assert!(looks_like_diff(b"diff --git a/x b/x\nindex 1..2\n"));
        assert!(looks_like_diff(b"--- a\n+++ b\n@@ -1 +1 @@\n"));
        assert!(!looks_like_diff(b"--- a\nplain\n"));
    }

    #[test]
    fn timestamps() {
        let len = |s: &str| timestamp_len(s.as_bytes());
        assert_eq!(len("2026-10-23T13:42:10.123Z x"), Some(24));
        assert_eq!(len("2026-10-23 13:42:10,5 x"), Some(21));
        assert_eq!(len("2026-10-23T13:42:10+02:00 x"), Some(25));
        assert_eq!(len("[2026-10-23 13:42:10] x"), Some(21));
        assert_eq!(len("Oct 23 13:42:10 host"), Some(15));
        assert_eq!(len("Oct  3 13:42:10 host"), Some(15));
        assert_eq!(len("13:42:10 x"), Some(8));
        assert_eq!(len("2026-10-23 only a date"), Some(10));
        assert_eq!(len("Octopus 23"), None);
        assert_eq!(len("hello"), None);
    }

    #[test]
    fn log_level_and_timestamp() {
        let line = "2026-10-23 13:42:10 ERROR db: connection lost";
        let spans = log_spans(line);
        assert_eq!(
            pieces(line, &spans),
            [
                ("2026-10-23 13:42:10", DIM),
                (" ", ""),
                ("ERROR", RED),
                (" db: connection lost", "")
            ]
        );
    }

    #[test]
    fn log_ignores_lowercase_prose_but_reads_level_fields() {
        let line = "2026-10-23 13:42:10 an error occurred";
        assert!(
            pieces(line, &log_spans(line))
                .iter()
                .all(|(_, s)| s != &RED)
        );
        let line = "ts=1 level=warn msg=x";
        assert!(pieces(line, &log_spans(line)).contains(&("warn", YELLOW)));
        let line = r#"{"time":"x","level":"error","msg":"ERROR in text"}"#;
        let got = pieces(line, &log_spans(line));
        assert!(got.contains(&("error", RED)));
        assert_eq!(got.iter().filter(|(_, s)| !s.is_empty()).count(), 1);
    }

    #[test]
    fn sniffs_logs() {
        assert!(looks_like_log(
            b"2026-10-23 13:42:10 a\n2026-10-23 13:42:11 b\n"
        ));
        assert!(!looks_like_log(b"hello\nworld\n"));
        assert!(!looks_like_log(b"2026-10-23 13:42:10 only one\nfoo\nbar\n"));
    }
}
