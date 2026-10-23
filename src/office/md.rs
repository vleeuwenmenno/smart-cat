//! Markdown building blocks shared by the converters: inline runs with formatting, and
//! a block writer that keeps blank lines, lists and tables well formed.

use super::{MAX_COLUMNS, MAX_MARKDOWN_BYTES};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Style {
    pub bold: bool,
    pub italic: bool,
    pub strike: bool,
}

struct Run {
    text: String,
    style: Style,
    link: Option<String>,
}

/// Text of one paragraph, collected run by run.
#[derive(Default)]
pub struct Inline {
    runs: Vec<Run>,
}

/// Plain text made safe: control characters dropped, line breaks and tabs turned into
/// spaces.
pub fn clean(text: &str) -> String {
    text.chars()
        .filter_map(|c| match c {
            '\n' | '\r' | '\t' | '\u{a0}' => Some(' '),
            c if c.is_control() => None,
            '\u{200b}' | '\u{feff}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' => None,
            c => Some(c),
        })
        .collect()
}

/// Escapes characters that markdown would read as formatting.
pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 8);
    for c in text.chars() {
        if matches!(
            c,
            '\\' | '`' | '*' | '_' | '[' | ']' | '<' | '>' | '|' | '~' | '&'
        ) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// Cleans and escapes text for a table cell.
pub fn cell_text(text: &str) -> String {
    escape(clean(text).trim())
}

/// Only these link targets are kept; anything else shows just the text.
fn safe_link(url: &str) -> bool {
    let lower = url.trim().to_ascii_lowercase();
    ["http://", "https://", "mailto:"]
        .iter()
        .any(|p| lower.starts_with(p))
        && !url.chars().any(|c| c.is_control() || c == ' ')
        && !url.contains(['(', ')', '<', '>'])
}

impl Inline {
    pub fn push(&mut self, text: &str, style: Style, link: Option<&str>) {
        let text = clean(text);
        if text.is_empty() {
            return;
        }
        let link = link.filter(|l| safe_link(l)).map(str::to_string);
        self.push_run(text, style, link);
    }

    fn push_run(&mut self, text: String, style: Style, link: Option<String>) {
        if let Some(last) = self.runs.last_mut()
            && last.style == style
            && last.link == link
            && last.text != "\n"
            && text != "\n"
        {
            last.text.push_str(&text);
            return;
        }
        self.runs.push(Run { text, style, link });
    }

    pub fn line_break(&mut self) {
        if !self.runs.is_empty() {
            self.push_run("\n".into(), Style::default(), None);
        }
    }

    pub fn is_blank(&self) -> bool {
        self.runs.iter().all(|r| r.text.trim().is_empty())
    }

    /// The markdown for the collected runs.
    pub fn finish(self) -> String {
        let mut out = String::new();
        let mut i = 0;
        while i < self.runs.len() {
            let link = self.runs[i].link.clone();
            let mut j = i;
            while j < self.runs.len() && self.runs[j].link == link {
                j += 1;
            }
            let mut segment = String::new();
            for run in &self.runs[i..j] {
                if run.text == "\n" {
                    segment.push_str("  \n");
                } else {
                    segment.push_str(&styled(&run.text, run.style));
                }
            }
            match link {
                Some(url) if !segment.trim().is_empty() => {
                    let core = segment.trim();
                    let start = segment.len() - segment.trim_start().len();
                    let end = segment.trim_end().len();
                    out.push_str(&format!(
                        "{}[{core}]({url}){}",
                        &segment[..start],
                        &segment[end..]
                    ));
                }
                _ => out.push_str(&segment),
            }
            i = j;
        }
        out.trim().to_string()
    }
}

fn styled(text: &str, style: Style) -> String {
    let core = text.trim();
    if core.is_empty() || style == Style::default() {
        return escape(text);
    }
    let start = text.len() - text.trim_start().len();
    let end = text.trim_end().len();
    let mut open = String::new();
    let mut close = String::new();
    if style.strike {
        open.push_str("~~");
        close.insert_str(0, "~~");
    }
    if style.italic {
        open.push('*');
        close.insert(0, '*');
    }
    if style.bold {
        open.push_str("**");
        close.insert_str(0, "**");
    }
    format!(
        "{}{open}{}{close}{}",
        &text[..start],
        escape(core),
        &text[end..]
    )
}

#[derive(PartialEq, Eq)]
enum Last {
    Start,
    Block,
    List,
}

/// Blocks in document order, written as markdown.
pub struct Md {
    out: String,
    last: Last,
    full: bool,
}

impl Md {
    pub fn new() -> Self {
        Self {
            out: String::new(),
            last: Last::Start,
            full: false,
        }
    }

    pub fn is_full(&self) -> bool {
        self.full
    }

    fn start_block(&mut self, list: bool) -> bool {
        if self.full {
            return false;
        }
        if self.out.len() >= MAX_MARKDOWN_BYTES {
            self.full = true;
            return false;
        }
        match (&self.last, list) {
            (Last::Start, _) => {}
            (Last::List, true) => self.out.push('\n'),
            _ => self.out.push_str("\n\n"),
        }
        self.last = if list { Last::List } else { Last::Block };
        true
    }

    pub fn heading(&mut self, level: usize, text: &str) {
        let text = text.trim();
        if text.is_empty() || !self.start_block(false) {
            return;
        }
        let text = text.replace('\n', " ");
        self.out.push_str(&"#".repeat(level.clamp(1, 6)));
        self.out.push(' ');
        self.out.push_str(&text);
    }

    pub fn paragraph(&mut self, text: &str) {
        let text = text.trim();
        if text.is_empty() || !self.start_block(false) {
            return;
        }
        self.out.push_str(&escape_block_start(text));
    }

    pub fn list_item(&mut self, depth: usize, ordered: bool, text: &str) {
        let text = text.trim();
        if text.is_empty() || !self.start_block(true) {
            return;
        }
        let depth = depth.min(8);
        self.out.push_str(&"   ".repeat(depth));
        self.out.push_str(if ordered { "1. " } else { "- " });
        self.out.push_str(&text.replace('\n', " "));
    }

    /// A table whose cells are already clean, escaped markdown. The first row is the header.
    pub fn table(&mut self, rows: &[Vec<String>]) {
        let columns = rows
            .iter()
            .map(Vec::len)
            .max()
            .unwrap_or(0)
            .min(MAX_COLUMNS);
        if columns == 0 || !self.start_block(false) {
            return;
        }
        let row = |cells: &[String]| {
            let mut line = String::from("|");
            for i in 0..columns {
                line.push(' ');
                line.push_str(cells.get(i).map(String::as_str).unwrap_or(""));
                line.push_str(" |");
            }
            line
        };
        self.out.push_str(&row(&rows[0]));
        self.out.push('\n');
        self.out.push('|');
        self.out.push_str(&" --- |".repeat(columns));
        for cells in &rows[1..] {
            self.out.push('\n');
            self.out.push_str(&row(cells));
        }
    }

    pub fn finish(mut self) -> String {
        if self.full {
            self.out
                .push_str("\n\n*… the rest of the document is not shown*");
        }
        self.out.push('\n');
        self.out
    }
}

/// Keeps a paragraph that starts like a heading, list or quote a paragraph.
fn escape_block_start(text: &str) -> String {
    let first = text.chars().next().unwrap_or(' ');
    if matches!(first, '#' | '+' | '-' | '=') {
        return format!("\\{text}");
    }
    let digits = text.chars().take_while(char::is_ascii_digit).count();
    if digits > 0 && matches!(text[digits..].chars().next(), Some('.' | ')')) {
        return format!("{}\\{}", &text[..digits], &text[digits..]);
    }
    text.to_string()
}

/// Collects table rows and cells while a parser walks the XML.
#[derive(Default)]
pub struct TableBuilder {
    rows: Vec<Vec<String>>,
    row: Vec<String>,
    cell: String,
}

impl TableBuilder {
    pub fn start_row(&mut self) {
        self.row.clear();
    }

    pub fn start_cell(&mut self) {
        self.cell.clear();
    }

    /// Adds a paragraph's markdown to the open cell.
    pub fn add(&mut self, text: &str) {
        let text = text.trim().replace('\n', " ");
        if text.is_empty() {
            return;
        }
        if !self.cell.is_empty() {
            self.cell.push(' ');
        }
        self.cell.push_str(&text);
    }

    pub fn end_cell(&mut self) {
        if self.row.len() < MAX_COLUMNS {
            self.row.push(std::mem::take(&mut self.cell));
        } else {
            self.cell.clear();
        }
    }

    /// Ends a cell that stands for `repeat` columns; only empty cells repeat.
    pub fn end_cell_repeated(&mut self, repeat: usize) {
        if self.cell.is_empty() {
            for _ in 0..repeat.max(1) {
                if self.row.len() < MAX_COLUMNS {
                    self.row.push(String::new());
                }
            }
        } else {
            self.end_cell();
        }
    }

    pub fn end_row(&mut self) {
        while self.row.last().is_some_and(String::is_empty) {
            self.row.pop();
        }
        if !self.row.is_empty() && self.rows.len() < 100_000 {
            self.rows.push(std::mem::take(&mut self.row));
        }
    }

    pub fn finish(self) -> Vec<Vec<String>> {
        self.rows
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLAIN: Style = Style {
        bold: false,
        italic: false,
        strike: false,
    };
    const BOLD: Style = Style {
        bold: true,
        ..PLAIN
    };

    #[test]
    fn merges_runs_and_keeps_spaces_outside_markers() {
        let mut inline = Inline::default();
        inline.push("Hello ", PLAIN, None);
        inline.push("big ", BOLD, None);
        inline.push("world", BOLD, None);
        inline.push("!", PLAIN, None);
        assert_eq!(inline.finish(), "Hello **big world**!");
    }

    #[test]
    fn escapes_and_cleans() {
        let mut inline = Inline::default();
        inline.push("a*b\x1b[31m|c", PLAIN, None);
        assert_eq!(inline.finish(), "a\\*b\\[31m\\|c");
    }

    #[test]
    fn unsafe_links_keep_only_the_text() {
        let mut inline = Inline::default();
        inline.push("click", PLAIN, Some("javascript:alert(1)"));
        inline.push(" ok", PLAIN, Some("https://example.com/a"));
        assert_eq!(inline.finish(), "click [ok](https://example.com/a)");
    }

    #[test]
    fn block_starts_are_escaped() {
        assert_eq!(escape_block_start("# not a heading"), "\\# not a heading");
        assert_eq!(escape_block_start("1. not a list"), "1\\. not a list");
    }

    #[test]
    fn lists_stay_tight() {
        let mut md = Md::new();
        md.heading(1, "T");
        md.list_item(0, false, "a");
        md.list_item(1, true, "b");
        md.paragraph("p");
        assert_eq!(md.finish(), "# T\n\n- a\n   1. b\n\np\n");
    }
}
