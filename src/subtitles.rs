//! Subtitle view: SubRip (`.srt`), WebVTT (`.vtt`) and SubStation Alpha (`.ass`, `.ssa`).
//!
//! An overview (format, cue count, span, language, warnings) comes first, then the cues
//! as `start → end  text`. At most [`MAX_READ_BYTES`] of the file are read; only the
//! first `max_cues` cues are kept for display, the rest are only counted. Cue text
//! comes from the file, so it is sanitized before it is shown.

use std::io::{self, Write};

use crate::kv;
use crate::render::sanitize_str;
use crate::util::human_size;

/// Most bytes read from one file.
pub const MAX_READ_BYTES: u64 = 16 * 1024 * 1024;
/// Cues longer than this many milliseconds are flagged.
const LONG_CUE_MS: i64 = 10_000;
/// Reading speed above this many characters per second is flagged.
const FAST_CPS: f64 = 25.0;
/// Text lines kept per cue.
const MAX_CUE_LINES: usize = 20;
/// Most characters kept per cue text line.
const MAX_LINE_CHARS: usize = 1000;

/// Private-use markers for italics inside cleaned cue text.
const ITALIC_ON: char = '\u{E000}';
const ITALIC_OFF: char = '\u{E001}';

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Srt,
    Vtt,
    Ass,
    Ssa,
}

impl Format {
    fn name(self) -> &'static str {
        match self {
            Format::Srt => "SubRip (srt)",
            Format::Vtt => "WebVTT",
            Format::Ass => "Advanced SubStation Alpha (ass)",
            Format::Ssa => "SubStation Alpha (ssa)",
        }
    }

    fn from_name(name: &str) -> Option<Format> {
        match name.to_ascii_lowercase().as_str() {
            "srt" | "subrip" => Some(Format::Srt),
            "vtt" | "webvtt" => Some(Format::Vtt),
            "ass" => Some(Format::Ass),
            "ssa" => Some(Format::Ssa),
            _ => None,
        }
    }
}

/// The subtitle format of a file, from a forced language, its extension, or its start.
pub fn wants(file_name: Option<&str>, language: Option<&str>, head: &[u8]) -> Option<Format> {
    if let Some(language) = language {
        return Format::from_name(language);
    }
    let ext = file_name
        .and_then(|n| n.rsplit_once('.'))
        .map(|(_, e)| e)
        .and_then(Format::from_name);
    if ext.is_some() {
        return ext;
    }
    let start = head.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(head);
    if start.starts_with(b"WEBVTT") {
        return Some(Format::Vtt);
    }
    if start.starts_with(b"[Script Info]") {
        return Some(Format::Ass);
    }
    None
}

pub struct Options<'a> {
    pub name: &'a str,
    pub colors: bool,
    pub utf8: bool,
    pub max_cues: usize,
    /// Cue numbers in a gutter (`-n`).
    pub line_numbers: bool,
}

#[derive(Debug, Clone, PartialEq)]
struct Cue {
    start: i64,
    end: i64,
    /// Style and actor (ass) or cue settings (vtt); empty when none.
    meta: String,
    /// Cleaned text; lines separated by `\n`, italics marked by private-use chars.
    text: String,
}

#[derive(Debug, Default)]
struct Parsed {
    language: Option<String>,
    title: Option<String>,
    styles: Option<usize>,
}

#[derive(Debug, Default)]
struct Summary {
    cues: u64,
    first: Option<i64>,
    last: Option<i64>,
    prev_start: Option<i64>,
    prev_max_end: Option<i64>,
    overlap: u64,
    out_of_order: u64,
    empty: u64,
    long: u64,
    fast: u64,
    bad_duration: u64,
    shown: Vec<Cue>,
}

impl Summary {
    fn add(&mut self, cue: Cue, max_cues: usize) {
        self.cues += 1;
        self.first = Some(self.first.map_or(cue.start, |f| f.min(cue.start)));
        self.last = Some(self.last.map_or(cue.end, |l| l.max(cue.end)));
        if let Some(prev) = self.prev_start {
            if cue.start < prev {
                self.out_of_order += 1;
            } else if self.prev_max_end.is_some_and(|end| cue.start < end) {
                self.overlap += 1;
            }
        }
        self.prev_start = Some(cue.start);
        self.prev_max_end = Some(self.prev_max_end.map_or(cue.end, |e| e.max(cue.end)));
        let chars = visible_chars(&cue.text);
        let duration = cue.end - cue.start;
        if chars == 0 {
            self.empty += 1;
        }
        if duration <= 0 {
            self.bad_duration += 1;
        } else {
            if duration > LONG_CUE_MS {
                self.long += 1;
            }
            if chars as f64 * 1000.0 / duration as f64 > FAST_CPS {
                self.fast += 1;
            }
        }
        if self.shown.len() < max_cues {
            self.shown.push(cue);
        }
    }
}

fn visible_chars(text: &str) -> usize {
    text.chars()
        .filter(|c| !matches!(*c, '\n' | ITALIC_ON | ITALIC_OFF))
        .count()
}

/// Decodes file bytes: UTF-8 (BOM dropped), UTF-16 with BOM, else Windows-1252.
/// Returns the text and a note when the file is not plain UTF-8.
fn decode(data: &[u8], truncated: bool) -> (String, Option<&'static str>) {
    if let Some(rest) = data.strip_prefix(b"\xEF\xBB\xBF") {
        return decode_utf8(rest, truncated);
    }
    let utf16 = |rest: &[u8], le: bool| {
        let units = rest.as_chunks::<2>().0.iter().map(|&c| {
            if le {
                u16::from_le_bytes(c)
            } else {
                u16::from_be_bytes(c)
            }
        });
        char::decode_utf16(units)
            .map(|r| r.unwrap_or('\u{FFFD}'))
            .collect::<String>()
    };
    if let Some(rest) = data.strip_prefix(b"\xFF\xFE") {
        return (utf16(rest, true), Some("UTF-16 LE, converted"));
    }
    if let Some(rest) = data.strip_prefix(b"\xFE\xFF") {
        return (utf16(rest, false), Some("UTF-16 BE, converted"));
    }
    decode_utf8(data, truncated)
}

fn decode_utf8(data: &[u8], truncated: bool) -> (String, Option<&'static str>) {
    match std::str::from_utf8(data) {
        Ok(text) => (text.to_string(), None),
        Err(err) if truncated && err.error_len().is_none() => {
            // A multi-byte character cut by the read cap.
            (
                String::from_utf8_lossy(&data[..err.valid_up_to()]).into_owned(),
                None,
            )
        }
        Err(_) => (
            data.iter().map(|&b| cp1252(b)).collect(),
            Some("not UTF-8, decoded as Windows-1252"),
        ),
    }
}

fn cp1252(byte: u8) -> char {
    const HIGH: [char; 32] = [
        '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8D}', 'Ž',
        '\u{8F}', '\u{90}', '‘', '’', '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ', '\u{9D}',
        'ž', 'Ÿ',
    ];
    match byte {
        0x80..=0x9F => HIGH[usize::from(byte - 0x80)],
        b => char::from(b),
    }
}

/// `[[H:]M:]S[.,]fraction` in milliseconds. The fraction is read as decimal digits, so
/// ass centiseconds (`.50`) and srt milliseconds (`,500`) both work.
fn parse_time(text: &str) -> Option<i64> {
    let text = text.trim();
    let (clock, fraction) = text.split_once(['.', ',']).unwrap_or((text, ""));
    if !fraction.bytes().all(|b| b.is_ascii_digit()) || fraction.len() > 9 {
        return None;
    }
    let parts: Vec<&str> = clock.split(':').collect();
    if !(2..=3).contains(&parts.len()) {
        return None;
    }
    let mut total: i64 = 0;
    for part in &parts {
        if part.is_empty() || part.len() > 6 || !part.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        total = total * 60 + part.parse::<i64>().ok()?;
    }
    let mut ms = 0;
    let mut scale = 100;
    for b in fraction.bytes().take(3) {
        ms += i64::from(b - b'0') * scale;
        scale /= 10;
    }
    Some(total * 1000 + ms)
}

fn format_time(ms: i64) -> String {
    let ms = ms.max(0);
    format!(
        "{:02}:{:02}:{:02}.{:03}",
        ms / 3_600_000,
        ms / 60_000 % 60,
        ms / 1000 % 60,
        ms % 1000
    )
}

fn format_span(ms: i64) -> String {
    let ms = ms.max(0);
    let (h, m, s) = (ms / 3_600_000, ms / 60_000 % 60, ms / 1000 % 60);
    if h > 0 {
        format!("{h} h {m} min {s} s")
    } else if m > 0 {
        format!("{m} min {s} s")
    } else {
        format!("{}.{:03} s", s, ms % 1000)
    }
}

fn decode_entities(text: &str) -> String {
    if !text.contains('&') {
        return text.to_string();
    }
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&nbsp;", " ")
        .replace("&lrm;", "")
        .replace("&rlm;", "")
        .replace("&amp;", "&")
}

/// One cue text line of srt or vtt: tags removed, `<i>` turned into italic markers.
fn clean_markup_line(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(pos) = rest.find(['<', '{']) {
        out.push_str(&rest[..pos]);
        let tail = &rest[pos..];
        let is_tag = tail.starts_with('<');
        let close = if is_tag { '>' } else { '}' };
        // `{...}` only counts as an override block when it starts with a backslash.
        let valid = is_tag || tail[1..].starts_with('\\');
        match tail.find(close).filter(|_| valid) {
            Some(end) => {
                if is_tag {
                    match tail[1..end].trim().to_ascii_lowercase().as_str() {
                        "i" => out.push(ITALIC_ON),
                        "/i" => out.push(ITALIC_OFF),
                        _ => {}
                    }
                }
                rest = &tail[end + 1..];
            }
            None => {
                out.push_str(&tail[..1]);
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    decode_entities(&out)
}

/// ass dialogue text: override blocks removed (`\i1`/`\i0` kept as italics), `\N` and
/// `\n` become line breaks, `\h` a space.
fn clean_ass(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '{' => {
                let mut block = String::new();
                for c in chars.by_ref() {
                    if c == '}' {
                        break;
                    }
                    block.push(c);
                }
                for tag in block.split('\\') {
                    match tag.trim() {
                        "i1" => out.push(ITALIC_ON),
                        "i0" => out.push(ITALIC_OFF),
                        _ => {}
                    }
                }
            }
            '\\' => match chars.peek() {
                Some('N' | 'n') => {
                    chars.next();
                    out.push('\n');
                }
                Some('h') => {
                    chars.next();
                    out.push(' ');
                }
                _ => out.push('\\'),
            },
            c => out.push(c),
        }
    }
    out
}

fn push_line(text: &mut String, lines: &mut usize, line: &str) {
    if *lines >= MAX_CUE_LINES {
        return;
    }
    if *lines > 0 {
        text.push('\n');
    }
    *lines += 1;
    let cleaned = clean_markup_line(line);
    text.extend(cleaned.chars().take(MAX_LINE_CHARS));
}

/// Characters that would collide with the italic markers are dropped from file text.
fn strip_markers(text: &str) -> std::borrow::Cow<'_, str> {
    if text.contains([ITALIC_ON, ITALIC_OFF]) {
        text.replace([ITALIC_ON, ITALIC_OFF], "").into()
    } else {
        text.into()
    }
}

/// `start --> end settings` as (start, end, settings).
fn parse_timing(line: &str) -> Option<(i64, i64, String)> {
    let (left, right) = line.split_once("-->")?;
    let start = parse_time(left)?;
    let right = right.trim_start();
    let (end, settings) = right.split_once(char::is_whitespace).unwrap_or((right, ""));
    Some((start, parse_time(end)?, settings.trim().to_string()))
}

fn parse_srt_vtt(
    text: &str,
    vtt: bool,
    on_cue: &mut dyn FnMut(Cue),
) -> Result<Parsed, &'static str> {
    let mut parsed = Parsed::default();
    let lines: Vec<&str> = text.lines().collect();
    let mut i = 0;
    if vtt {
        let first = lines
            .first()
            .map_or("", |l| l.trim_start_matches('\u{FEFF}'));
        if !first.starts_with("WEBVTT") {
            return Err("missing WEBVTT header");
        }
        // Header lines run to the first blank line, or to the first timing line.
        i = 1;
        while i < lines.len() && !lines[i].trim().is_empty() && !lines[i].contains("-->") {
            if let Some((key, value)) = lines[i].split_once(':')
                && key.trim().eq_ignore_ascii_case("language")
            {
                parsed.language = Some(value.trim().to_string());
            }
            i += 1;
        }
    }
    let mut current: Option<(Cue, usize)> = None;
    let mut skipping = false;
    let mut block_start = true;
    let mut count = 0usize;
    while i < lines.len() {
        let line = lines[i].trim_end_matches('\r');
        i += 1;
        if line.trim().is_empty() {
            if let Some((cue, _)) = current.take() {
                on_cue(cue);
                count += 1;
            }
            skipping = false;
            block_start = true;
            continue;
        }
        let was_block_start = std::mem::replace(&mut block_start, false);
        if skipping {
            continue;
        }
        if vtt
            && current.is_none()
            && was_block_start
            && ["NOTE", "STYLE", "REGION"].iter().any(|k| {
                line.strip_prefix(k)
                    .is_some_and(|r| r.is_empty() || r.starts_with([' ', '\t']))
            })
        {
            skipping = true;
            continue;
        }
        if let Some((start, end, settings)) = parse_timing(line) {
            if let Some((cue, _)) = current.take() {
                on_cue(cue);
                count += 1;
            }
            let meta = if vtt { settings } else { String::new() };
            current = Some((
                Cue {
                    start,
                    end,
                    meta: strip_markers(&meta).into_owned(),
                    text: String::new(),
                },
                0,
            ));
            continue;
        }
        if let Some((cue, kept)) = current.as_mut() {
            // A bare number right before a timing line is the next cue's index.
            let next_is_timing = lines.get(i).is_some_and(|n| parse_timing(n).is_some());
            if line.bytes().all(|b| b.is_ascii_digit()) && next_is_timing {
                continue;
            }
            push_line(&mut cue.text, kept, &strip_markers(line));
        }
    }
    if let Some((cue, _)) = current.take() {
        on_cue(cue);
        count += 1;
    }
    if count == 0 {
        return Err("no cues found");
    }
    Ok(parsed)
}

fn parse_ass(text: &str, on_cue: &mut dyn FnMut(Cue)) -> Result<Parsed, &'static str> {
    let mut parsed = Parsed::default();
    let mut section = String::new();
    let mut styles = 0usize;
    let mut format: Vec<String> = ["layer", "start", "end", "style", "name", "text"]
        .iter()
        .map(|s| (*s).to_string())
        .collect();
    let mut seen_events = false;
    let mut count = 0usize;
    for line in text.lines() {
        let line = line.trim_start_matches('\u{FEFF}').trim();
        if line.is_empty() || line.starts_with(';') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            section = line[1..line.len() - 1].to_ascii_lowercase();
            seen_events |= section == "events";
            continue;
        }
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim().to_ascii_lowercase();
        let value = value.trim();
        match (section.as_str(), key.as_str()) {
            ("script info", "title") => parsed.title = Some(value.to_string()),
            ("script info", "language") => parsed.language = Some(value.to_string()),
            (s, "style") if s.contains("styles") => styles += 1,
            ("events", "format") => {
                format = value
                    .split(',')
                    .map(|f| f.trim().to_ascii_lowercase())
                    .collect();
            }
            ("events", "dialogue") => {
                let fields: Vec<&str> = value.splitn(format.len().max(1), ',').collect();
                let get = |name: &str| {
                    format
                        .iter()
                        .position(|f| f == name)
                        .and_then(|p| fields.get(p))
                        .map_or("", |s| s.trim())
                };
                let (Some(start), Some(end)) = (parse_time(get("start")), parse_time(get("end")))
                else {
                    continue;
                };
                let style = get("style");
                let actor = get("name");
                let mut meta = Vec::new();
                if !style.is_empty() && !style.eq_ignore_ascii_case("default") {
                    meta.push(style);
                }
                if !actor.is_empty() {
                    meta.push(actor);
                }
                let cleaned = clean_ass(&strip_markers(get("text")));
                let text: Vec<String> = cleaned
                    .split('\n')
                    .take(MAX_CUE_LINES)
                    .map(|l| decode_entities(l).chars().take(MAX_LINE_CHARS).collect())
                    .collect();
                on_cue(Cue {
                    start,
                    end,
                    meta: strip_markers(&meta.join(" / ")).into_owned(),
                    text: text.join("\n"),
                });
                count += 1;
            }
            _ => {}
        }
    }
    parsed.styles = Some(styles);
    if !seen_events {
        return Err("missing [Events] section");
    }
    if count == 0 {
        return Err("no dialogue lines found");
    }
    Ok(parsed)
}

/// A language tag from a name like `movie.en.srt` or `movie.pt-BR.srt`.
fn language_from_name(file_name: &str) -> Option<String> {
    let stem = file_name.rsplit_once('.')?.0;
    let tag = stem.rsplit_once('.')?.1;
    let simple = (2..=3).contains(&tag.len()) && tag.bytes().all(|b| b.is_ascii_alphabetic());
    let regional = tag.len() == 5
        && tag.as_bytes()[2] == b'-'
        && tag[..2].bytes().all(|b| b.is_ascii_alphabetic())
        && tag[3..].bytes().all(|b| b.is_ascii_alphabetic());
    (simple || regional).then(|| tag.to_string())
}

/// What went wrong with a subtitle file; the caller falls back to the text view.
pub enum Failure {
    Io(io::Error),
    /// The file is not a subtitle file of the expected format.
    Invalid(&'static str),
}

impl From<io::Error> for Failure {
    fn from(err: io::Error) -> Self {
        Failure::Io(err)
    }
}

/// Writes the view for `data` (already read, at most [`MAX_READ_BYTES`]). Nothing is
/// written when the file does not parse.
pub fn render<W: Write>(
    out: &mut W,
    format: Format,
    data: &[u8],
    truncated: bool,
    file_name: Option<&str>,
    opts: &Options,
) -> Result<(), Failure> {
    let (text, encoding) = decode(data, truncated);
    let mut summary = Summary::default();
    let mut add = |cue: Cue| summary.add(cue, opts.max_cues);
    let parsed = match format {
        Format::Srt => parse_srt_vtt(&text, false, &mut add),
        Format::Vtt => parse_srt_vtt(&text, true, &mut add),
        Format::Ass | Format::Ssa => parse_ass(&text, &mut add),
    }
    .map_err(Failure::Invalid)?;

    let mut rows: Vec<(&str, String)> = vec![
        ("format", format.name().to_string()),
        ("cues", crate::table::thousands(summary.cues)),
    ];
    if let (Some(first), Some(last)) = (summary.first, summary.last) {
        rows.push(("first", format_time(first)));
        rows.push(("last", format_time(last)));
        rows.push(("span", format_span(last - first)));
    }
    let language = parsed
        .language
        .clone()
        .filter(|l| !l.is_empty())
        .map(|l| (l, false))
        .or_else(|| file_name.and_then(language_from_name).map(|l| (l, true)));
    if let Some((language, guessed)) = language {
        rows.push((
            "language",
            if guessed {
                format!("{language} (from the file name)")
            } else {
                language
            },
        ));
    }
    if matches!(format, Format::Ass | Format::Ssa) {
        if let Some(title) = &parsed.title {
            rows.push(("title", title.clone()));
        }
        if let Some(styles) = parsed.styles {
            rows.push(("styles", styles.to_string()));
        }
    }
    if let Some(encoding) = encoding {
        rows.push(("encoding", encoding.to_string()));
    }
    if truncated {
        rows.push((
            "note",
            format!("only the first {} were read", human_size(MAX_READ_BYTES)),
        ));
    }
    let count = |n: u64| format!("{n} cue{}", if n == 1 { "" } else { "s" });
    if summary.overlap > 0 {
        rows.push((
            "overlapping",
            format!(
                "{} start before the previous one ends",
                count(summary.overlap)
            ),
        ));
    }
    if summary.out_of_order > 0 {
        rows.push((
            "out of order",
            format!(
                "{} start before the previous one",
                count(summary.out_of_order)
            ),
        ));
    }
    if summary.empty > 0 {
        rows.push(("empty", count(summary.empty)));
    }
    if summary.bad_duration > 0 {
        rows.push((
            "bad duration",
            format!(
                "{} end at or before their start",
                count(summary.bad_duration)
            ),
        ));
    }
    if summary.long > 0 {
        rows.push((
            "too long",
            format!("{} last over {} s", count(summary.long), LONG_CUE_MS / 1000),
        ));
    }
    if summary.fast > 0 {
        rows.push((
            "fast reading",
            format!(
                "{} above {FAST_CPS} characters per second",
                count(summary.fast)
            ),
        ));
    }

    let mut buf = Vec::new();
    kv::write_table(&mut buf, opts.name, &rows, opts.colors)?;
    buf.push(b'\n');
    write_cues(&mut buf, &summary, opts)?;
    out.write_all(&buf)?;
    Ok(())
}

fn write_cues<W: Write>(out: &mut W, summary: &Summary, opts: &Options) -> io::Result<()> {
    let (dim, undim, italic, unitalic) = if opts.colors {
        ("\x1b[2m", "\x1b[22m", "\x1b[3m", "\x1b[23m")
    } else {
        ("", "", "", "")
    };
    let (arrow, bar) = if opts.utf8 {
        ("→", "│")
    } else {
        ("->", "|")
    };
    let time_width = 12 * 2 + arrow.chars().count() + 2 + 2;
    for (index, cue) in summary.shown.iter().enumerate() {
        let (gutter, blank_gutter) = if opts.line_numbers {
            (
                format!("{dim}{:>4} {bar}{undim} ", index + 1),
                format!("{dim}     {bar}{undim} "),
            )
        } else {
            (String::new(), String::new())
        };
        let meta = if cue.meta.is_empty() {
            String::new()
        } else {
            format!("{dim}[{}]{undim} ", sanitize_str(&cue.meta))
        };
        let mut italic_on = false;
        for (n, line) in cue.text.split('\n').enumerate() {
            if n == 0 {
                write!(
                    out,
                    "{gutter}{dim}{} {arrow} {}{undim}  {meta}",
                    format_time(cue.start),
                    format_time(cue.end)
                )?;
            } else {
                write!(out, "{blank_gutter}{:time_width$}", "")?;
                if italic_on {
                    write!(out, "{italic}")?;
                }
            }
            let mut segment = String::new();
            for c in line.chars() {
                match c {
                    ITALIC_ON | ITALIC_OFF => {
                        out.write_all(sanitize_str(&segment).as_bytes())?;
                        segment.clear();
                        italic_on = c == ITALIC_ON;
                        write!(out, "{}", if italic_on { italic } else { unitalic })?;
                    }
                    c => segment.push(c),
                }
            }
            out.write_all(sanitize_str(&segment).as_bytes())?;
            if italic_on {
                write!(out, "{unitalic}")?;
            }
            writeln!(out)?;
        }
    }
    let hidden = summary.cues - summary.shown.len() as u64;
    if hidden > 0 {
        writeln!(
            out,
            "{dim}… {} more cue{}{undim}",
            crate::table::thousands(hidden),
            if hidden == 1 { "" } else { "s" }
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times() {
        assert_eq!(parse_time("00:01:02,500"), Some(62_500));
        assert_eq!(parse_time("01:02.5"), Some(62_500));
        assert_eq!(parse_time("0:00:01.50"), Some(1_500));
        assert_eq!(parse_time("nope"), None);
        assert_eq!(format_time(3_723_004), "01:02:03.004");
    }

    #[test]
    fn language_guess() {
        assert_eq!(language_from_name("movie.en.srt").as_deref(), Some("en"));
        assert_eq!(
            language_from_name("movie.pt-BR.srt").as_deref(),
            Some("pt-BR")
        );
        assert_eq!(language_from_name("movie.srt"), None);
    }

    #[test]
    fn markup_and_ass_cleaning() {
        assert_eq!(
            clean_markup_line("<i>hi</i> <c.x>there</c> {\\an8}&amp; 1 < 2"),
            format!("{ITALIC_ON}hi{ITALIC_OFF} there & 1 < 2")
        );
        assert_eq!(
            clean_ass("{\\an8\\i1}one{\\i0}\\Ntwo\\hthree"),
            format!("{ITALIC_ON}one{ITALIC_OFF}\ntwo three")
        );
    }

    #[test]
    fn windows_1252() {
        let (text, note) = decode(b"caf\xE9 \x93x\x94", false);
        assert_eq!(text, "café “x”");
        assert!(note.is_some());
    }
}
