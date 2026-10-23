//! CSV, TSV and PSV files as box-drawn tables fitted to the terminal.
//!
//! The first record is the header. Only the first `max_rows` data rows are read into
//! memory; the rest are counted while streaming, so huge files stay cheap.

use std::io::{self, Read, Write};

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::render::sanitize_str;

/// A cell is kept at most this many characters long; the screen never shows more.
const MAX_CELL_CHARS: usize = 512;
/// Narrowest a column gets squeezed to (one character and the ellipsis).
const MIN_COLUMN: usize = 2;
/// Rows past the shown ones are counted only this far into the file.
const COUNT_LIMIT_BYTES: u64 = 64 * 1024 * 1024;
/// Delimiters `.csv` files are sniffed for.
const CANDIDATES: [u8; 4] = *b",;\t|";

/// How the field delimiter of a file is chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Delimiter {
    /// Sniff one of `,` `;` tab `|` from the start of the file.
    Sniff,
    Fixed(u8),
}

/// The delimiter choice for a file name or forced language, or `None` when the file is
/// not a delimited-text file.
pub fn delimiter_for(file_name: Option<&str>, language: Option<&str>) -> Option<Delimiter> {
    let kind = match language {
        Some(language) => language.to_ascii_lowercase(),
        None => {
            let (stem, ext) = file_name?.rsplit_once('.')?;
            if stem.is_empty() {
                return None;
            }
            ext.to_ascii_lowercase()
        }
    };
    match kind.as_str() {
        "csv" => Some(Delimiter::Sniff),
        "tsv" => Some(Delimiter::Fixed(b'\t')),
        "psv" => Some(Delimiter::Fixed(b'|')),
        _ => None,
    }
}

pub struct Options {
    pub width: usize,
    pub colors: bool,
    pub utf8: bool,
    pub line_numbers: bool,
    /// Data rows shown; 0 means all of them.
    pub max_rows: usize,
}

/// Picks the delimiter that splits the first records into the most consistent number
/// of fields. Comma wins ties and is the answer when nothing splits.
pub fn sniff(head: &[u8], truncated: bool) -> u8 {
    let mut best = (0usize, 0usize);
    let mut chosen = b',';
    for delimiter in CANDIDATES {
        let mut reader = csv::ReaderBuilder::new()
            .delimiter(delimiter)
            .has_headers(false)
            .flexible(true)
            .from_reader(head);
        let mut counts: Vec<usize> = reader
            .byte_records()
            .take(40)
            .map_while(Result::ok)
            .map(|r| r.len())
            .collect();
        if truncated {
            counts.pop();
        }
        let Some(&modal) = counts
            .iter()
            .max_by_key(|&&c| (counts.iter().filter(|&&x| x == c).count(), c))
        else {
            continue;
        };
        if modal < 2 {
            continue;
        }
        let agreeing = counts.iter().filter(|&&c| c == modal).count();
        let score = (agreeing * 100 / counts.len(), modal);
        if score > best {
            best = score;
            chosen = delimiter;
        }
    }
    chosen
}

struct Cell {
    text: String,
    width: usize,
    numeric: bool,
}

/// Renders the file read from `reader`. `head` is the already peeked start of it, used
/// for delimiter sniffing only.
pub fn render<R: Read, W: Write>(
    reader: R,
    head: &[u8],
    delimiter: Delimiter,
    opts: &Options,
    out: &mut W,
) -> io::Result<()> {
    let delimiter = match delimiter {
        Delimiter::Fixed(d) => d,
        Delimiter::Sniff => sniff(head, head.len() >= crate::detect::SNIFF_BYTES),
    };
    let mut records = csv::ReaderBuilder::new()
        .delimiter(delimiter)
        .has_headers(false)
        .flexible(true)
        .from_reader(reader);

    let mut record = csv::ByteRecord::new();
    let mut rows: Vec<Vec<Cell>> = Vec::new();
    let mut more_rows = 0u64;
    let mut at_least = false;
    while records.read_byte_record(&mut record)? {
        // rows[0] is the header, then at most max_rows data rows.
        if opts.max_rows == 0 || rows.len() <= opts.max_rows {
            let mut row: Vec<Cell> = record.iter().map(cell).collect();
            if rows.is_empty()
                && let Some(first) = row.first_mut()
                && first.text.starts_with('\u{feff}')
            {
                *first = make_cell(first.text.trim_start_matches('\u{feff}').to_string());
            }
            rows.push(row);
        } else {
            more_rows += 1;
            if records.position().byte() > COUNT_LIMIT_BYTES {
                at_least = true;
                break;
            }
        }
    }
    if rows.is_empty() {
        return Ok(());
    }
    out.write_all(&layout(&rows, more_rows, at_least, opts))
}

/// Draws ready-made rows (the first is the header) as a box table. `more_rows` is the
/// number of data rows left out, for the footer. Cells are sanitized here.
pub fn render_rows<W: Write>(
    rows: &[Vec<String>],
    more_rows: u64,
    opts: &Options,
    out: &mut W,
) -> io::Result<()> {
    if rows.is_empty() {
        return Ok(());
    }
    let cells: Vec<Vec<Cell>> = rows
        .iter()
        .map(|row| row.iter().map(|text| make_cell(text.clone())).collect())
        .collect();
    out.write_all(&layout(&cells, more_rows, false, opts))
}

fn cell(bytes: &[u8]) -> Cell {
    let text = String::from_utf8_lossy(bytes);
    let text: String = if text.chars().nth(MAX_CELL_CHARS).is_some() {
        text.chars().take(MAX_CELL_CHARS).collect()
    } else {
        text.into_owned()
    };
    make_cell(text)
}

fn make_cell(raw: String) -> Cell {
    let numeric = is_number(&raw);
    // Line breaks inside quoted fields would break the grid.
    let flat = raw.replace("\r\n", "↵").replace(['\n', '\r'], "↵");
    let text = sanitize_str(&flat);
    let width = text.width();
    Cell {
        text,
        width,
        numeric,
    }
}

fn is_number(text: &str) -> bool {
    let t = text.trim();
    t.starts_with(|c: char| c.is_ascii_digit() || matches!(c, '+' | '-' | '.'))
        && t.parse::<f64>().is_ok()
}

fn layout(rows: &[Vec<Cell>], more_rows: u64, at_least: bool, opts: &Options) -> Vec<u8> {
    let columns = rows.iter().map(Vec::len).max().unwrap_or(0).max(1);
    let data_rows = rows.len() - 1;

    // Numeric columns: at least one number below the header and nothing else but blanks.
    let numeric: Vec<bool> = (0..columns)
        .map(|c| {
            let mut any = false;
            for row in &rows[1..] {
                match row.get(c) {
                    Some(cell) if cell.numeric => any = true,
                    Some(cell) if !cell.text.trim().is_empty() => return false,
                    _ => {}
                }
            }
            any
        })
        .collect();

    let gutter = opts
        .line_numbers
        .then(|| data_rows.max(1).to_string().len());

    // Columns that fit at their narrowest; the rest are summarised in the footer.
    let fixed = 1 + gutter.map_or(0, |g| g + 3);
    let fit = (opts.width.saturating_sub(fixed) / (MIN_COLUMN + 3)).max(1);
    let shown = columns.min(fit);

    let mut widths: Vec<usize> = (0..shown)
        .map(|c| {
            rows.iter()
                .filter_map(|r| r.get(c))
                .map(|cell| cell.width)
                .max()
                .unwrap_or(0)
                .max(1)
        })
        .collect();
    let budget = opts
        .width
        .saturating_sub(fixed + 3 * shown)
        .max(MIN_COLUMN * shown);
    fit_widths(&mut widths, budget);

    let glyphs = if opts.utf8 { UNICODE } else { ASCII };
    let (dim, undim, bold, unbold) = if opts.colors {
        ("\x1b[2m", "\x1b[22m", "\x1b[1m", "\x1b[22m")
    } else {
        ("", "", "", "")
    };
    let border = |s: &str| format!("{dim}{s}{undim}");
    let rule = |left: &str, mid: &str, right: &str| {
        let spans = gutter
            .map(|g| g + 2)
            .into_iter()
            .chain(widths.iter().map(|w| w + 2))
            .map(|span| glyphs.h.repeat(span))
            .collect::<Vec<_>>();
        let mut line = border(&format!("{left}{}{right}", spans.join(mid)));
        line.push('\n');
        line
    };

    let mut out = String::new();
    out.push_str(&rule(glyphs.tl, glyphs.top, glyphs.tr));
    for (index, row) in rows.iter().enumerate() {
        out.push_str(&border(glyphs.v));
        if let Some(g) = gutter {
            let label = if index == 0 {
                " ".repeat(g)
            } else {
                format!("{index:>g$}")
            };
            out.push_str(&format!(" {dim}{label}{undim} "));
            out.push_str(&border(glyphs.v));
        }
        for (c, &width) in widths.iter().enumerate() {
            let (text, cell_width) = match row.get(c) {
                Some(cell) => fitted(&cell.text, cell.width, width, glyphs.ellipsis),
                None => (String::new(), 0),
            };
            let pad = " ".repeat(width.saturating_sub(cell_width));
            let (before, after) = if numeric[c] {
                (pad, String::new())
            } else {
                (String::new(), pad)
            };
            let (open, close) = if index == 0 { (bold, unbold) } else { ("", "") };
            out.push_str(&format!(" {before}{open}{text}{close}{after} "));
            out.push_str(&border(glyphs.v));
        }
        out.push('\n');
        if index == 0 {
            out.push_str(&rule(glyphs.ml, glyphs.cross, glyphs.mr));
        }
    }
    out.push_str(&rule(glyphs.bl, glyphs.bottom, glyphs.br));

    let mut notes = Vec::new();
    if more_rows > 0 {
        let plus = if at_least { "+" } else { "" };
        let s = if more_rows == 1 && !at_least { "" } else { "s" };
        notes.push(format!("{}{plus} more row{s}", thousands(more_rows)));
    }
    if shown < columns {
        let hidden = columns - shown;
        let s = if hidden == 1 { "" } else { "s" };
        notes.push(format!("{hidden} more column{s}"));
    }
    if !notes.is_empty() {
        out.push_str(&format!(
            "{dim}{} {}{undim}\n",
            glyphs.ellipsis,
            notes.join(", ")
        ));
    }
    out.into_bytes()
}

/// Shrinks the widest columns until the total fits `budget`, then hands out what is
/// left over to the columns that had to be cut.
fn fit_widths(widths: &mut [usize], budget: usize) {
    if widths.iter().sum::<usize>() <= budget {
        return;
    }
    let natural = widths.to_vec();
    let mut cap = natural.iter().copied().max().unwrap_or(0);
    while cap > MIN_COLUMN && natural.iter().map(|&w| w.min(cap)).sum::<usize>() > budget {
        cap -= 1;
    }
    let mut left = budget.saturating_sub(natural.iter().map(|&w| w.min(cap)).sum());
    for (width, &want) in widths.iter_mut().zip(&natural) {
        *width = want.min(cap);
    }
    for (width, &want) in widths.iter_mut().zip(&natural) {
        if left > 0 && want > *width {
            *width += 1;
            left -= 1;
        }
    }
}

/// `text` cut to `max` columns with an ellipsis when it is too wide.
fn fitted(text: &str, width: usize, max: usize, ellipsis: &str) -> (String, usize) {
    if width <= max {
        return (text.to_string(), width);
    }
    let mut out = String::new();
    let mut used = 0;
    for ch in text.chars() {
        let w = ch.width().unwrap_or(0);
        if used + w + 1 > max {
            break;
        }
        out.push(ch);
        used += w;
    }
    out.push_str(ellipsis);
    (out, used + 1)
}

pub(crate) fn thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

struct Glyphs {
    h: &'static str,
    v: &'static str,
    tl: &'static str,
    tr: &'static str,
    bl: &'static str,
    br: &'static str,
    top: &'static str,
    bottom: &'static str,
    ml: &'static str,
    mr: &'static str,
    cross: &'static str,
    ellipsis: &'static str,
}

const UNICODE: Glyphs = Glyphs {
    h: "─",
    v: "│",
    tl: "┌",
    tr: "┐",
    bl: "└",
    br: "┘",
    top: "┬",
    bottom: "┴",
    ml: "├",
    mr: "┤",
    cross: "┼",
    ellipsis: "…",
};

const ASCII: Glyphs = Glyphs {
    h: "-",
    v: "|",
    tl: "+",
    tr: "+",
    bl: "+",
    br: "+",
    top: "+",
    bottom: "+",
    ml: "+",
    mr: "+",
    cross: "+",
    ellipsis: "~",
};

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(width: usize) -> Options {
        Options {
            width,
            colors: false,
            utf8: true,
            line_numbers: false,
            max_rows: 200,
        }
    }

    fn table(text: &str, delimiter: Delimiter, opts: &Options) -> String {
        let mut out = Vec::new();
        let head = &text.as_bytes()[..text.len().min(8192)];
        render(text.as_bytes(), head, delimiter, opts, &mut out).unwrap();
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn lays_out_header_and_numeric_alignment() {
        let got = table(
            "name,qty,note\napple,3,red\nfig,12345,\n",
            Delimiter::Sniff,
            &opts(80),
        );
        assert_eq!(
            got,
            "\
┌───────┬───────┬──────┐
│ name  │   qty │ note │
├───────┼───────┼──────┤
│ apple │     3 │ red  │
│ fig   │ 12345 │      │
└───────┴───────┴──────┘
"
        );
    }

    #[test]
    fn sniffs_delimiters() {
        assert_eq!(sniff(b"a;b;c\n1;2;3\n", false), b';');
        assert_eq!(sniff(b"a\tb\n1\t2\n", false), b'\t');
        assert_eq!(sniff(b"a|b|c\n1|2|3\n", false), b'|');
        assert_eq!(sniff(b"a,b\n1,2\n", false), b',');
        // Commas inside prose do not beat a consistent semicolon layout.
        assert_eq!(sniff(b"x;y\nhello, world;2\nfoo;3\n", false), b';');
        // Nothing splits: comma.
        assert_eq!(sniff(b"one\ntwo\n", false), b',');
        // A cut-off last record is ignored.
        assert_eq!(sniff(b"a;b;c\n1;2;3\n4;5", true), b';');
    }

    #[test]
    fn picks_delimiters_by_extension() {
        assert_eq!(delimiter_for(Some("a.CSV"), None), Some(Delimiter::Sniff));
        assert_eq!(
            delimiter_for(Some("a.tsv"), None),
            Some(Delimiter::Fixed(b'\t'))
        );
        assert_eq!(
            delimiter_for(Some("a.psv"), None),
            Some(Delimiter::Fixed(b'|'))
        );
        assert_eq!(delimiter_for(Some("a.txt"), None), None);
        assert_eq!(delimiter_for(Some(".csv"), None), None);
        assert_eq!(
            delimiter_for(Some("a.txt"), Some("csv")),
            Some(Delimiter::Sniff)
        );
        assert_eq!(delimiter_for(None, None), None);
    }

    #[test]
    fn handles_quotes_and_embedded_newlines() {
        let got = table(
            "a,b\n\"x, y\",\"say \"\"hi\"\"\"\n\"two\nlines\",z\n",
            Delimiter::Sniff,
            &opts(80),
        );
        assert_eq!(
            got,
            "\
┌───────────┬──────────┐
│ a         │ b        │
├───────────┼──────────┤
│ x, y      │ say \"hi\" │
│ two↵lines │ z        │
└───────────┴──────────┘
"
        );
    }

    #[test]
    fn tsv_is_split_on_tabs_only() {
        let got = table("a b\tc\n1,2\t3\n", Delimiter::Fixed(b'\t'), &opts(80));
        assert!(got.contains("│ a b │ c │"), "{got}");
        assert!(got.contains("│ 1,2 │ 3 │"), "{got}");
    }

    #[test]
    fn truncates_wide_cells_and_never_wraps() {
        let got = table(
            "id,text\n1,abcdefghijklmnopqrstuvwxyz\n",
            Delimiter::Sniff,
            &opts(20),
        );
        for line in got.lines() {
            assert_eq!(line.width(), 20, "{line}");
        }
        assert!(got.contains("│ abcdefghij… │"), "{got}");
    }

    #[test]
    fn hides_columns_that_cannot_fit() {
        let got = table("a,b,c,d,e,f\n1,2,3,4,5,6\n", Delimiter::Sniff, &opts(16));
        assert!(got.ends_with("… 3 more columns\n"), "{got}");
        assert!(got.lines().all(|l| l.width() <= 16), "{got}");
    }

    #[test]
    fn footer_counts_rows_past_the_limit() {
        let mut text = String::from("n\n");
        for i in 0..1300 {
            text.push_str(&format!("{i}\n"));
        }
        let mut o = opts(80);
        o.max_rows = 5;
        let got = table(&text, Delimiter::Sniff, &o);
        assert_eq!(got.lines().count(), 3 + 5 + 1 + 1, "{got}");
        assert!(got.ends_with("… 1,295 more rows\n"), "{got}");
        o.max_rows = 0;
        assert!(!table(&text, Delimiter::Sniff, &o).contains("more row"));
    }

    #[test]
    fn row_gutter_and_colours() {
        let mut o = opts(80);
        o.line_numbers = true;
        let got = table("a\nx\ny\n", Delimiter::Sniff, &o);
        assert_eq!(
            got,
            "\
┌───┬───┐
│   │ a │
├───┼───┤
│ 1 │ x │
│ 2 │ y │
└───┴───┘
"
        );
        o.colors = true;
        o.line_numbers = false;
        let got = table("a\nx\n", Delimiter::Sniff, &o);
        assert!(got.contains("\x1b[1ma\x1b[22m"), "{got:?}");
        assert!(got.contains("\x1b[2m│\x1b[22m"), "{got:?}");
    }

    #[test]
    fn ascii_glyphs_and_sanitized_cells() {
        let mut o = opts(80);
        o.utf8 = false;
        let got = table("a\n\x1b]0;x\x07\n", Delimiter::Fixed(b','), &o);
        assert!(got.starts_with('+'), "{got}");
        assert!(!got.contains('\x1b'));
        assert!(got.contains("^[]0;x^G"), "{got}");
    }

    #[test]
    fn ragged_rows_and_bom() {
        let got = table("\u{feff}a,b\n1\n1,2,3\n", Delimiter::Sniff, &opts(80));
        assert!(got.contains("│ a "), "{got}");
        assert_eq!(got.lines().next().unwrap().matches('┬').count(), 2, "{got}");
    }

    #[test]
    fn formats_thousands() {
        assert_eq!(thousands(7), "7");
        assert_eq!(thousands(12345), "12,345");
        assert_eq!(thousands(1000000), "1,000,000");
    }
}
