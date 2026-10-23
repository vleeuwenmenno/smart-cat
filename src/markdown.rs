//! Markdown rendering: headings, wrapped paragraphs, lists, block quotes, tables,
//! highlighted code blocks and links, drawn with SGR styles and box-drawing characters.
//!
//! Everything taken from the document is untrusted. Text goes through the sanitizer
//! before it is measured or printed, and link targets are percent-encoded before they
//! are put in an OSC 8 hyperlink.

use std::borrow::Cow;
use std::io;
use std::path::{Path, PathBuf};

use pulldown_cmark::{
    Alignment, BlockQuoteKind, CodeBlockKind, Event, HeadingLevel, Options as ParseOptions, Parser,
    Tag, TagEnd,
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::image::Drawn;
use crate::render::{sanitize_line, sanitize_str};
use crate::term::ColorDepth;

/// Rendered text never gets wider than this, however wide the terminal is.
pub const MAX_WIDTH: usize = 100;
/// Columns that code blocks are indented by.
const CODE_INDENT: &str = "    ";

/// File extensions rendered as markdown.
const EXTENSIONS: &[&str] = &["md", "markdown", "mdown", "mkd"];

/// True when a file should be rendered as markdown: a markdown extension, or a forced
/// language (`-l md`, a rule's `language`) naming markdown.
pub fn is_markdown(file_name: Option<&str>, language: Option<&str>) -> bool {
    if let Some(language) = language {
        return language.eq_ignore_ascii_case("markdown") || language.eq_ignore_ascii_case("md");
    }
    file_name
        .and_then(|name| name.rsplit_once('.'))
        .is_some_and(|(stem, ext)| {
            !stem.is_empty() && EXTENSIONS.iter().any(|e| e.eq_ignore_ascii_case(ext))
        })
}

/// The width to render at: the terminal's columns, else `$COLUMNS`, else 80; at most
/// [`MAX_WIDTH`].
pub fn render_width(stdout_tty: bool) -> usize {
    let columns = stdout_tty
        .then(|| rustix::termios::tcgetwinsize(io::stdout()).ok())
        .flatten()
        .map(|size| usize::from(size.ws_col))
        .filter(|&cols| cols > 0)
        .or_else(|| {
            std::env::var("COLUMNS")
                .ok()
                .and_then(|v| v.trim().parse::<usize>().ok())
                .filter(|&cols| cols > 0)
        })
        .unwrap_or(80);
    columns.min(MAX_WIDTH)
}

pub struct Options {
    pub width: usize,
    pub colors: bool,
    pub depth: ColorDepth,
    pub utf8: bool,
    /// Wrap link text in OSC 8 hyperlinks (only meaningful with colours on).
    pub hyperlinks: bool,
    /// The terminal background is light; picks the inline code colours.
    pub light: bool,
    pub tab_width: usize,
}

/// Highlights a code block: `(language, code)` to one encoded line per source line,
/// without newlines. `None` means "show it plain".
pub type Highlighter<'a> = &'a mut dyn FnMut(&str, &str) -> Option<Vec<Vec<u8>>>;

/// Draws an image: `(destination, max columns)` to the encoded image, or `None` to
/// keep the `[image: alt]` placeholder.
pub type ImageLoader<'a> = &'a mut dyn FnMut(&str, u16) -> Option<Drawn>;

/// At most this many images are drawn per document; later ones keep the placeholder.
const MAX_IMAGES: usize = 50;

/// Images in a document resolve to local files only: no scheme (`http:`, `data:`,
/// `file:`, ...) and no `//host` form. A query or fragment is dropped, and a relative
/// path is taken from `base` (the markdown file's directory). Returns the path when it
/// names a regular file. Reading any local file is fine: the user could `cat` it, and
/// it is only decoded as an image, never printed as text.
pub fn local_image_path(base: &Path, dest: &str) -> Option<PathBuf> {
    let dest = dest.trim();
    if dest.is_empty() || dest.starts_with("//") {
        return None;
    }
    if let Some((scheme, _)) = dest.split_once(':')
        && !scheme.is_empty()
        && scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
        && scheme.starts_with(|c: char| c.is_ascii_alphabetic())
    {
        return None;
    }
    let dest = dest.split(['?', '#']).next().unwrap_or_default();
    if dest.is_empty() {
        return None;
    }
    let candidates = [Some(dest.to_string()), percent_decode(dest)];
    candidates.into_iter().flatten().find_map(|d| {
        let path = base.join(d);
        path.is_file().then_some(path)
    })
}

/// Decodes `%XX` escapes; `None` when there are none or the result is not UTF-8.
fn percent_decode(text: &str) -> Option<String> {
    if !text.contains('%') {
        return None;
    }
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(h), Some(l)) = (hex(bytes[i + 1]), hex(bytes[i + 2]))
        {
            out.push((h * 16 + l) as u8);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// Renders a markdown document to bytes ready for the terminal. Images keep their
/// placeholder; see [`render_with_images`].
#[cfg_attr(not(test), allow(dead_code))]
pub fn render(text: &str, opts: &Options, highlight: Highlighter) -> Vec<u8> {
    let mut no_images = |_: &str, _: u16| None;
    render_with_images(text, opts, highlight, &mut no_images)
}

/// Renders a markdown document, drawing images through `images` where it returns one.
pub fn render_with_images(
    text: &str,
    opts: &Options,
    highlight: Highlighter,
    images: ImageLoader,
) -> Vec<u8> {
    let mut parse = ParseOptions::empty();
    parse.insert(ParseOptions::ENABLE_TABLES);
    parse.insert(ParseOptions::ENABLE_STRIKETHROUGH);
    parse.insert(ParseOptions::ENABLE_TASKLISTS);
    parse.insert(ParseOptions::ENABLE_FOOTNOTES);
    parse.insert(ParseOptions::ENABLE_GFM);
    parse.insert(ParseOptions::ENABLE_YAML_STYLE_METADATA_BLOCKS);

    let mut renderer = Renderer::new(opts, highlight, images);
    for event in Parser::new_ext(text, parse) {
        renderer.event(event);
    }
    renderer.finish()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct Style {
    bold: bool,
    dim: bool,
    italic: bool,
    underline: bool,
    strike: bool,
    fg: Option<&'static str>,
    bg: Option<&'static str>,
    /// Index into the renderer's sanitized link targets.
    link: Option<usize>,
}

impl Style {
    fn visual(self) -> Self {
        Self { link: None, ..self }
    }

    fn sgr(self, out: &mut Vec<u8>) {
        out.extend_from_slice(b"\x1b[0");
        for (on, code) in [
            (self.bold, ";1"),
            (self.dim, ";2"),
            (self.italic, ";3"),
            (self.underline, ";4"),
            (self.strike, ";9"),
        ] {
            if on {
                out.extend_from_slice(code.as_bytes());
            }
        }
        for color in [self.fg, self.bg].into_iter().flatten() {
            out.push(b';');
            out.extend_from_slice(color.as_bytes());
        }
        out.push(b'm');
    }
}

fn dim() -> Style {
    Style {
        dim: true,
        ..Style::default()
    }
}

/// A run of text in one style, already sanitized.
#[derive(Debug, Clone, PartialEq)]
struct Piece {
    text: String,
    style: Style,
}

impl Piece {
    fn new(text: impl Into<String>, style: Style) -> Self {
        Self {
            text: text.into(),
            style,
        }
    }
}

/// Inline content waiting to be wrapped.
#[derive(Debug, Clone)]
enum Inline {
    Text(String, Style),
    Break,
}

struct Palette {
    headings: [Option<&'static str>; 6],
    code_fg: &'static str,
    code_bg: Option<&'static str>,
    link: &'static str,
    marker: &'static str,
    checked: &'static str,
    footnote: &'static str,
}

impl Palette {
    fn new(opts: &Options) -> Self {
        let rich = opts.depth != ColorDepth::Ansi16;
        let (code_fg, code_bg) = match (rich, opts.light) {
            (false, _) => ("33", None),
            (true, false) => ("38;5;215", Some("48;5;236")),
            (true, true) => ("38;5;130", Some("48;5;254")),
        };
        Self {
            headings: [
                Some("95"),
                Some("94"),
                Some("96"),
                Some("92"),
                Some("93"),
                None,
            ],
            code_fg,
            code_bg,
            link: "94",
            marker: "36",
            checked: "32",
            footnote: "36",
        }
    }
}

struct Glyphs {
    bullets: &'static [&'static str],
    unchecked: &'static str,
    checked: &'static str,
    quote: &'static str,
    rule: &'static str,
    h1_rule: &'static str,
    h2_rule: &'static str,
    /// Table borders: top (left, mid, right), separator, bottom, vertical, horizontal.
    top: [&'static str; 3],
    sep: [&'static str; 3],
    bottom: [&'static str; 3],
    vertical: &'static str,
    horizontal: &'static str,
}

const UTF8_GLYPHS: Glyphs = Glyphs {
    bullets: &["•", "◦", "▪"],
    unchecked: "☐",
    checked: "☑",
    quote: "│",
    rule: "─",
    h1_rule: "━",
    h2_rule: "─",
    top: ["┌", "┬", "┐"],
    sep: ["├", "┼", "┤"],
    bottom: ["└", "┴", "┘"],
    vertical: "│",
    horizontal: "─",
};

const ASCII_GLYPHS: Glyphs = Glyphs {
    bullets: &["*", "-"],
    unchecked: "[ ]",
    checked: "[x]",
    quote: "|",
    rule: "-",
    h1_rule: "=",
    h2_rule: "-",
    top: ["+", "+", "+"],
    sep: ["+", "+", "+"],
    bottom: ["+", "+", "+"],
    vertical: "|",
    horizontal: "-",
};

/// Block containers that put something in front of every line.
enum Container {
    Quote(Style),
    /// A list item or footnote definition: a marker on its first line, then a hanging
    /// indent of `indent` columns.
    Item {
        marker: Piece,
        indent: usize,
        used: bool,
    },
}

/// Output for the document body or for the footnotes collected at its end.
#[derive(Default)]
struct Sink {
    buf: Vec<u8>,
    started: bool,
    pending_blank: bool,
}

struct Table {
    alignments: Vec<Alignment>,
    rows: Vec<Vec<Vec<Inline>>>,
    row: Vec<Vec<Inline>>,
    head_rows: usize,
}

struct Renderer<'o, 'h> {
    opts: &'o Options,
    palette: Palette,
    glyphs: &'static Glyphs,
    highlight: Highlighter<'h>,
    images: ImageLoader<'h>,
    images_drawn: usize,
    sink: Sink,
    footnotes: Sink,
    containers: Vec<Container>,
    /// Next number for ordered lists, `None` for bullet lists.
    lists: Vec<Option<u64>>,
    style: Style,
    styles: Vec<Style>,
    inline: Vec<Inline>,
    links: Vec<String>,
    /// Destination and start index in `inline` of each open link.
    open_links: Vec<(String, usize)>,
    image_alt: Option<String>,
    image_url: String,
    image_depth: usize,
    heading: Option<HeadingLevel>,
    code: Option<(String, String)>,
    html: Option<String>,
    table: Option<Table>,
}

impl<'o, 'h> Renderer<'o, 'h> {
    fn new(opts: &'o Options, highlight: Highlighter<'h>, images: ImageLoader<'h>) -> Self {
        Self {
            opts,
            palette: Palette::new(opts),
            glyphs: if opts.utf8 {
                &UTF8_GLYPHS
            } else {
                &ASCII_GLYPHS
            },
            highlight,
            images,
            images_drawn: 0,
            sink: Sink::default(),
            footnotes: Sink::default(),
            containers: Vec::new(),
            lists: Vec::new(),
            style: Style::default(),
            styles: Vec::new(),
            inline: Vec::new(),
            links: Vec::new(),
            open_links: Vec::new(),
            image_alt: None,
            image_url: String::new(),
            image_depth: 0,
            heading: None,
            code: None,
            html: None,
            table: None,
        }
    }

    fn push_style(&mut self, change: impl FnOnce(&mut Style)) {
        self.styles.push(self.style);
        change(&mut self.style);
    }

    fn pop_style(&mut self) {
        if let Some(style) = self.styles.pop() {
            self.style = style;
        }
    }

    fn event(&mut self, event: Event) {
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(text) => self.text(&text),
            Event::Code(code) => {
                let style = Style {
                    fg: Some(self.palette.code_fg),
                    bg: self.palette.code_bg,
                    ..self.style
                };
                let code = clean(&code);
                if self.image_depth > 0 {
                    self.text(&code);
                } else if self.opts.colors {
                    self.inline.push(Inline::Text(code.into_owned(), style));
                } else {
                    self.inline.push(Inline::Text(format!("`{code}`"), style));
                }
            }
            Event::Html(html) => {
                if let Some(buf) = self.html.as_mut() {
                    buf.push_str(&html);
                } else {
                    self.inline_html(&html);
                }
            }
            Event::InlineHtml(html) => self.inline_html(&html),
            Event::FootnoteReference(label) => {
                let style = Style {
                    fg: Some(self.palette.footnote),
                    ..self.style
                };
                self.inline
                    .push(Inline::Text(format!("[{}]", clean(&label)), style));
            }
            Event::SoftBreak => self.text(" "),
            Event::HardBreak => {
                if self.image_depth == 0 {
                    self.inline.push(Inline::Break);
                }
            }
            Event::Rule => {
                self.flush_paragraph();
                self.begin_block();
                let width = self.avail();
                self.line(&[Piece::new(self.glyphs.rule.repeat(width), dim())]);
                self.sink.pending_blank = true;
            }
            Event::TaskListMarker(checked) => {
                let (glyph, color) = if checked {
                    (self.glyphs.checked, self.palette.checked)
                } else {
                    (self.glyphs.unchecked, self.palette.marker)
                };
                if let Some(Container::Item { marker, indent, .. }) = self.containers.last_mut() {
                    *marker = Piece::new(
                        glyph,
                        Style {
                            fg: Some(color),
                            ..Style::default()
                        },
                    );
                    *indent = glyph.width() + 1;
                }
            }
            Event::InlineMath(_) | Event::DisplayMath(_) => {}
        }
    }

    fn text(&mut self, text: &str) {
        if let Some((_, code)) = self.code.as_mut() {
            code.push_str(text);
        } else if let Some(html) = self.html.as_mut() {
            html.push_str(text);
        } else if let Some(alt) = self.image_alt.as_mut() {
            alt.push_str(text);
        } else {
            self.inline
                .push(Inline::Text(clean(text).into_owned(), self.style));
        }
    }

    fn inline_html(&mut self, html: &str) {
        let tag = html.trim().to_ascii_lowercase();
        if matches!(tag.as_str(), "<br>" | "<br/>" | "<br />") {
            self.inline.push(Inline::Break);
            return;
        }
        let style = Style {
            dim: true,
            ..self.style
        };
        self.inline
            .push(Inline::Text(clean(html).into_owned(), style));
    }

    fn start(&mut self, tag: Tag) {
        match tag {
            Tag::Paragraph => self.flush_paragraph(),
            Tag::Heading { level, .. } => {
                self.flush_paragraph();
                let fg = self.palette.headings[level as usize - 1];
                self.push_style(|s| {
                    s.bold = true;
                    s.fg = fg;
                });
                if level as usize > 2 {
                    let marker = format!("{} ", "#".repeat(level as usize));
                    self.inline.push(Inline::Text(marker, self.style));
                }
                self.heading = Some(level);
            }
            Tag::BlockQuote(kind) => {
                self.flush_paragraph();
                self.blank_if_pending();
                let color = kind.map(|kind| match kind {
                    BlockQuoteKind::Note => "94",
                    BlockQuoteKind::Tip => "92",
                    BlockQuoteKind::Important => "95",
                    BlockQuoteKind::Warning => "93",
                    BlockQuoteKind::Caution => "91",
                });
                let bar = Style {
                    dim: color.is_none(),
                    fg: color,
                    ..Style::default()
                };
                self.containers.push(Container::Quote(bar));
                if let Some(kind) = kind {
                    let label = match kind {
                        BlockQuoteKind::Note => "Note",
                        BlockQuoteKind::Tip => "Tip",
                        BlockQuoteKind::Important => "Important",
                        BlockQuoteKind::Warning => "Warning",
                        BlockQuoteKind::Caution => "Caution",
                    };
                    let style = Style { bold: true, ..bar };
                    self.line(&[Piece::new(label, style)]);
                }
            }
            Tag::CodeBlock(kind) => {
                self.flush_paragraph();
                let lang = match kind {
                    CodeBlockKind::Fenced(info) => info
                        .split(|c: char| c.is_whitespace() || c == ',' || c == '{')
                        .next()
                        .unwrap_or("")
                        .to_string(),
                    CodeBlockKind::Indented => String::new(),
                };
                self.code = Some((lang, String::new()));
            }
            Tag::MetadataBlock(_) => {
                self.flush_paragraph();
                self.code = Some(("yaml".into(), String::new()));
            }
            Tag::HtmlBlock => {
                self.flush_paragraph();
                self.html = Some(String::new());
            }
            Tag::List(start) => {
                self.flush_paragraph();
                self.lists.push(start);
            }
            Tag::Item => {
                self.flush_paragraph();
                let depth = self.lists.len().saturating_sub(1);
                let text = match self.lists.last_mut() {
                    Some(Some(number)) => {
                        let text = format!("{number}.");
                        *number += 1;
                        text
                    }
                    _ => {
                        let bullets = self.glyphs.bullets;
                        bullets[depth % bullets.len()].to_string()
                    }
                };
                let indent = text.width() + 1;
                let style = Style {
                    fg: Some(self.palette.marker),
                    ..Style::default()
                };
                self.containers.push(Container::Item {
                    marker: Piece::new(text, style),
                    indent,
                    used: false,
                });
            }
            Tag::FootnoteDefinition(label) => {
                self.flush_paragraph();
                std::mem::swap(&mut self.sink, &mut self.footnotes);
                let text = format!("[{}]", clean(&label));
                let indent = text.width() + 1;
                let style = Style {
                    fg: Some(self.palette.footnote),
                    ..Style::default()
                };
                self.containers.push(Container::Item {
                    marker: Piece::new(text, style),
                    indent,
                    used: false,
                });
            }
            Tag::Table(alignments) => {
                self.flush_paragraph();
                self.table = Some(Table {
                    alignments,
                    rows: Vec::new(),
                    row: Vec::new(),
                    head_rows: 0,
                });
            }
            Tag::TableHead => self.push_style(|s| s.bold = true),
            Tag::TableRow | Tag::TableCell => self.inline.clear(),
            Tag::Emphasis => self.push_style(|s| s.italic = true),
            Tag::Strong => self.push_style(|s| s.bold = true),
            Tag::Strikethrough => self.push_style(|s| s.strike = true),
            Tag::Link { dest_url, .. } => {
                let link = if self.opts.colors && self.opts.hyperlinks {
                    self.links.push(hyperlink_target(&dest_url));
                    Some(self.links.len() - 1)
                } else {
                    None
                };
                let fg = self.palette.link;
                self.push_style(|s| {
                    s.underline = true;
                    s.fg = Some(fg);
                    s.link = link;
                });
                self.open_links
                    .push((dest_url.into_string(), self.inline.len()));
            }
            Tag::Image { dest_url, .. } => {
                self.image_depth += 1;
                if self.image_depth == 1 {
                    self.image_alt = Some(String::new());
                    self.image_url = dest_url.into_string();
                }
            }
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph => {
                self.flush_paragraph();
                self.sink.pending_blank = true;
            }
            TagEnd::Heading(level) => {
                self.pop_style();
                self.heading = None;
                let inline = std::mem::take(&mut self.inline);
                self.begin_block();
                let width = self.avail();
                for line in wrap(&inline, width) {
                    self.line(&line);
                }
                let rule = match level {
                    HeadingLevel::H1 => Some(self.glyphs.h1_rule),
                    HeadingLevel::H2 => Some(self.glyphs.h2_rule),
                    _ => None,
                };
                if let Some(rule) = rule {
                    let style = Style {
                        fg: self.palette.headings[level as usize - 1],
                        ..Style::default()
                    };
                    self.line(&[Piece::new(rule.repeat(width), style)]);
                }
                self.sink.pending_blank = true;
            }
            TagEnd::BlockQuote(_) => {
                self.flush_paragraph();
                self.containers.pop();
                self.sink.pending_blank = true;
            }
            TagEnd::CodeBlock | TagEnd::MetadataBlock(_) => {
                if let Some((lang, code)) = self.code.take() {
                    self.code_block(&lang, &code);
                }
            }
            TagEnd::HtmlBlock => {
                if let Some(html) = self.html.take() {
                    self.html_block(&html);
                }
            }
            TagEnd::List(_) => {
                self.flush_paragraph();
                self.lists.pop();
                if !self
                    .containers
                    .iter()
                    .any(|c| matches!(c, Container::Item { .. }))
                {
                    self.sink.pending_blank = true;
                }
            }
            TagEnd::Item => {
                self.flush_paragraph();
                self.close_item();
            }
            TagEnd::FootnoteDefinition => {
                self.flush_paragraph();
                self.close_item();
                self.sink.pending_blank = true;
                std::mem::swap(&mut self.sink, &mut self.footnotes);
            }
            TagEnd::TableHead => {
                self.pop_style();
                self.end_row();
                if let Some(table) = self.table.as_mut() {
                    table.head_rows = table.rows.len();
                }
            }
            TagEnd::TableRow => self.end_row(),
            TagEnd::TableCell => {
                let cell = std::mem::take(&mut self.inline);
                if let Some(table) = self.table.as_mut() {
                    table.row.push(cell);
                }
            }
            TagEnd::Table => {
                if let Some(table) = self.table.take() {
                    self.render_table(table);
                }
            }
            TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough => self.pop_style(),
            TagEnd::Link => {
                self.pop_style();
                if let Some((dest, start)) = self.open_links.pop() {
                    let text: String = self.inline[start.min(self.inline.len())..]
                        .iter()
                        .filter_map(|i| match i {
                            Inline::Text(t, _) => Some(t.as_str()),
                            Inline::Break => None,
                        })
                        .collect();
                    let shown = clean(&dest);
                    let redundant = dest.is_empty()
                        || text == shown
                        || dest.strip_prefix("mailto:") == Some(text.as_str());
                    if !redundant && self.image_depth == 0 {
                        self.inline.push(Inline::Text(" ".into(), Style::default()));
                        self.inline.push(Inline::Text(
                            format!("({shown})"),
                            Style {
                                dim: true,
                                ..self.style
                            },
                        ));
                    }
                }
            }
            TagEnd::Image => {
                self.image_depth = self.image_depth.saturating_sub(1);
                if self.image_depth == 0
                    && let Some(alt) = self.image_alt.take()
                {
                    if self.draw_image() {
                        return;
                    }
                    let alt = clean(alt.trim());
                    let text = if alt.is_empty() {
                        "[image]".to_string()
                    } else {
                        format!("[image: {alt}]")
                    };
                    let style = Style {
                        dim: true,
                        ..self.style
                    };
                    self.inline.push(Inline::Text(text, style));
                }
            }
            _ => {}
        }
    }

    /// Draws the image that just ended as a block of its own, after the text before
    /// it. Images in headings and tables keep their placeholder.
    fn draw_image(&mut self) -> bool {
        if self.table.is_some() || self.heading.is_some() || self.images_drawn >= MAX_IMAGES {
            return false;
        }
        let url = std::mem::take(&mut self.image_url);
        let max_cols = u16::try_from(self.avail()).unwrap_or(u16::MAX);
        let Some(drawn) = (self.images)(&url, max_cols) else {
            return false;
        };
        self.images_drawn += 1;
        self.flush_paragraph();
        self.begin_block();
        if drawn.is_pixel() {
            let pieces = self.prefix();
            write_pieces(&mut self.sink.buf, &pieces, self.opts, &self.links);
            self.sink.buf.extend_from_slice(&drawn.data);
            self.sink.buf.push(b'\n');
        } else {
            for line in drawn.lines() {
                let pieces = self.prefix();
                write_pieces(&mut self.sink.buf, &pieces, self.opts, &self.links);
                self.sink.buf.extend_from_slice(line);
                self.sink.buf.push(b'\n');
            }
        }
        self.sink.started = true;
        true
    }

    fn finish(mut self) -> Vec<u8> {
        self.flush_paragraph();
        if !self.footnotes.buf.is_empty() {
            self.begin_block();
            let width = self.avail().min(20);
            self.line(&[Piece::new(self.glyphs.rule.repeat(width), dim())]);
            let footnotes = std::mem::take(&mut self.footnotes.buf);
            self.sink.buf.extend_from_slice(&footnotes);
        }
        self.sink.buf
    }

    fn end_row(&mut self) {
        if let Some(table) = self.table.as_mut() {
            let row = std::mem::take(&mut table.row);
            if !row.is_empty() {
                table.rows.push(row);
            }
        }
    }

    fn close_item(&mut self) {
        if let Some(Container::Item { used: false, .. }) = self.containers.last() {
            self.line(&[]);
        }
        self.containers.pop();
    }

    /// Columns available for content inside the current containers.
    fn avail(&self) -> usize {
        let used: usize = self
            .containers
            .iter()
            .map(|c| match c {
                Container::Quote(_) => 2,
                Container::Item { indent, .. } => *indent,
            })
            .sum();
        self.opts.width.saturating_sub(used).max(10)
    }

    fn blank_if_pending(&mut self) {
        if self.sink.pending_blank && self.sink.started {
            self.blank();
        }
        self.sink.pending_blank = false;
    }

    fn begin_block(&mut self) {
        self.blank_if_pending();
    }

    /// Wraps and writes pending inline text (a paragraph, or the text of a tight list item).
    fn flush_paragraph(&mut self) {
        if self.table.is_some() || self.heading.is_some() {
            return;
        }
        if self.inline.iter().all(|i| match i {
            Inline::Text(t, _) => t.trim().is_empty(),
            Inline::Break => true,
        }) {
            self.inline.clear();
            return;
        }
        let inline = std::mem::take(&mut self.inline);
        self.begin_block();
        for line in wrap(&inline, self.avail()) {
            self.line(&line);
        }
    }

    fn prefix(&mut self) -> Vec<Piece> {
        let quote = self.glyphs.quote;
        let mut pieces = Vec::new();
        for container in &mut self.containers {
            match container {
                Container::Quote(style) => pieces.push(Piece::new(format!("{quote} "), *style)),
                Container::Item {
                    marker,
                    indent,
                    used,
                } => {
                    if *used {
                        pieces.push(Piece::new(" ".repeat(*indent), Style::default()));
                    } else {
                        *used = true;
                        pieces.push(marker.clone());
                        let pad = indent.saturating_sub(marker.text.width());
                        pieces.push(Piece::new(" ".repeat(pad), Style::default()));
                    }
                }
            }
        }
        pieces
    }

    fn line(&mut self, content: &[Piece]) {
        let mut pieces = self.prefix();
        pieces.extend_from_slice(content);
        write_pieces(&mut self.sink.buf, &pieces, self.opts, &self.links);
        self.sink.buf.push(b'\n');
        self.sink.started = true;
    }

    fn raw_line(&mut self, bytes: &[u8]) {
        let pieces = self.prefix();
        write_pieces(&mut self.sink.buf, &pieces, self.opts, &self.links);
        self.sink.buf.extend_from_slice(CODE_INDENT.as_bytes());
        self.sink.buf.extend_from_slice(bytes);
        self.sink.buf.push(b'\n');
        self.sink.started = true;
    }

    /// An empty line: only the quote bars of the enclosing block quotes.
    fn blank(&mut self) {
        let quote = self.glyphs.quote;
        let mut pieces = Vec::new();
        let mut pad = 0;
        for container in &self.containers {
            match container {
                Container::Quote(style) => {
                    if pad > 0 {
                        pieces.push(Piece::new(" ".repeat(pad), Style::default()));
                    }
                    pieces.push(Piece::new(quote, *style));
                    pad = 1;
                }
                Container::Item { indent, .. } => pad += indent,
            }
        }
        write_pieces(&mut self.sink.buf, &pieces, self.opts, &self.links);
        self.sink.buf.push(b'\n');
    }

    fn code_block(&mut self, lang: &str, code: &str) {
        self.begin_block();
        let highlighted = if self.opts.colors && !lang.is_empty() {
            (self.highlight)(lang, code)
        } else {
            None
        };
        match highlighted {
            Some(lines) => {
                for line in lines {
                    self.raw_line(&line);
                }
            }
            None => {
                for line in code.lines() {
                    let line = sanitize_line(line, self.opts.tab_width);
                    self.raw_line(line.as_bytes());
                }
            }
        }
        self.sink.pending_blank = true;
    }

    fn html_block(&mut self, html: &str) {
        let trimmed = html.trim();
        if trimmed.is_empty()
            || (trimmed.starts_with("<!--")
                && trimmed.ends_with("-->")
                && trimmed[4..].find("-->") == Some(trimmed.len() - 7))
        {
            return;
        }
        self.begin_block();
        for line in trimmed.lines() {
            let line = sanitize_line(line, self.opts.tab_width);
            self.line(&[Piece::new(line, dim())]);
        }
        self.sink.pending_blank = true;
    }

    fn render_table(&mut self, table: Table) {
        let columns = table
            .rows
            .iter()
            .map(Vec::len)
            .max()
            .unwrap_or(0)
            .max(table.alignments.len());
        if columns == 0 {
            return;
        }
        self.begin_block();

        // Natural width of each column: its widest cell laid out on one line.
        let mut widths = vec![1usize; columns];
        for row in &table.rows {
            for (col, cell) in row.iter().enumerate() {
                let natural = wrap(cell, usize::MAX)
                    .iter()
                    .map(|line| line_width(line))
                    .max()
                    .unwrap_or(0);
                widths[col] = widths[col].max(natural);
            }
        }
        // Shrink the widest column until the table fits; cells wrap within it.
        let borders = columns * 3 + 1;
        let budget = self.avail().saturating_sub(borders);
        const FLOOR: usize = 4;
        while widths.iter().sum::<usize>() > budget {
            let (col, &widest) = widths
                .iter()
                .enumerate()
                .max_by_key(|&(i, w)| (*w, std::cmp::Reverse(i)))
                .expect("at least one column");
            if widest <= FLOOR {
                break;
            }
            widths[col] -= 1;
        }

        let laid_out: Vec<Vec<Vec<Vec<Piece>>>> = table
            .rows
            .iter()
            .map(|row| {
                (0..columns)
                    .map(|col| row.get(col).map_or_else(Vec::new, |c| wrap(c, widths[col])))
                    .collect()
            })
            .collect();
        let tall_rows = laid_out
            .iter()
            .skip(table.head_rows)
            .any(|row| row.iter().any(|cell| cell.len() > 1));

        let g = self.glyphs;
        let border = dim();
        let rule = |ends: [&str; 3]| {
            let mut text = String::from(ends[0]);
            for (i, w) in widths.iter().enumerate() {
                if i > 0 {
                    text.push_str(ends[1]);
                }
                text.push_str(&g.horizontal.repeat(w + 2));
            }
            text.push_str(ends[2]);
            Piece::new(text, border)
        };
        let top = rule(g.top);
        let sep = rule(g.sep);
        let bottom = rule(g.bottom);

        self.line(&[top]);
        for (index, row) in laid_out.iter().enumerate() {
            if index > 0 && (index == table.head_rows || (tall_rows && index > table.head_rows)) {
                self.line(std::slice::from_ref(&sep));
            }
            let height = row.iter().map(Vec::len).max().unwrap_or(0).max(1);
            for line_index in 0..height {
                let mut pieces = vec![Piece::new(format!("{} ", g.vertical), border)];
                for (col, cell) in row.iter().enumerate() {
                    let content = cell.get(line_index).map_or(&[][..], Vec::as_slice);
                    let free = widths[col].saturating_sub(line_width(content));
                    let align = table
                        .alignments
                        .get(col)
                        .copied()
                        .unwrap_or(Alignment::None);
                    let left = match align {
                        Alignment::Right => free,
                        Alignment::Center => free / 2,
                        Alignment::Left | Alignment::None => 0,
                    };
                    if left > 0 {
                        pieces.push(Piece::new(" ".repeat(left), Style::default()));
                    }
                    pieces.extend_from_slice(content);
                    if free > left {
                        pieces.push(Piece::new(" ".repeat(free - left), Style::default()));
                    }
                    let edge = if col + 1 == columns {
                        format!(" {}", g.vertical)
                    } else {
                        format!(" {} ", g.vertical)
                    };
                    pieces.push(Piece::new(edge, border));
                }
                self.line(&pieces);
            }
        }
        self.line(&[bottom]);
        self.sink.pending_blank = true;
    }
}

/// Writes styled pieces; every line starts and ends in the default style.
fn write_pieces(out: &mut Vec<u8>, pieces: &[Piece], opts: &Options, links: &[String]) {
    let mut current = Style::default();
    let mut link = None;
    for piece in pieces {
        if piece.text.is_empty() {
            continue;
        }
        if opts.colors {
            if piece.style.link != link {
                if link.is_some() {
                    out.extend_from_slice(b"\x1b]8;;\x1b\\");
                }
                if let Some(target) = piece.style.link.and_then(|i| links.get(i)) {
                    out.extend_from_slice(b"\x1b]8;;");
                    out.extend_from_slice(target.as_bytes());
                    out.extend_from_slice(b"\x1b\\");
                }
                link = piece.style.link;
            }
            if piece.style.visual() != current {
                piece.style.visual().sgr(out);
                current = piece.style.visual();
            }
        }
        out.extend_from_slice(piece.text.as_bytes());
    }
    if link.is_some() {
        out.extend_from_slice(b"\x1b]8;;\x1b\\");
    }
    if current != Style::default() {
        out.extend_from_slice(b"\x1b[0m");
    }
}

fn line_width(line: &[Piece]) -> usize {
    line.iter().map(|p| p.text.width()).sum()
}

/// Sanitizes document text for display; tabs become single spaces.
fn clean(text: &str) -> Cow<'_, str> {
    let unsafe_byte = |(i, b): (usize, &u8)| {
        *b < 0x20
            || *b == 0x7f
            || (*b == 0xc2
                && text
                    .as_bytes()
                    .get(i + 1)
                    .is_some_and(|n| (0x80..=0x9f).contains(n)))
    };
    if text.as_bytes().iter().enumerate().any(unsafe_byte) {
        Cow::Owned(sanitize_str(text))
    } else {
        Cow::Borrowed(text)
    }
}

/// A link target that is safe inside an OSC 8 sequence: every byte outside printable
/// ASCII (controls, spaces, non-ASCII) is percent-encoded.
fn hyperlink_target(url: &str) -> String {
    let mut out = String::with_capacity(url.len());
    for &byte in url.as_bytes() {
        if (0x21..=0x7e).contains(&byte) {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// Greedy word wrap of styled inline text into lines at most `width` columns wide.
/// Words longer than a line are split between characters.
fn wrap(inline: &[Inline], width: usize) -> Vec<Vec<Piece>> {
    let mut wrapper = Wrapper {
        width: width.max(1),
        lines: Vec::new(),
        line: Vec::new(),
        line_width: 0,
        space: None,
        word: Vec::new(),
        word_width: 0,
    };
    for item in inline {
        match item {
            Inline::Break => {
                wrapper.flush_word();
                wrapper.new_line();
            }
            Inline::Text(text, style) => {
                for ch in text.chars() {
                    if ch == ' ' {
                        wrapper.flush_word();
                        if wrapper.line_width > 0 {
                            wrapper.space.get_or_insert(*style);
                        }
                    } else {
                        push_char(&mut wrapper.word, ch, *style);
                        wrapper.word_width += ch.width().unwrap_or(0);
                    }
                }
            }
        }
    }
    wrapper.flush_word();
    if !wrapper.line.is_empty() || wrapper.lines.is_empty() {
        wrapper.new_line();
    }
    wrapper.lines
}

fn push_char(pieces: &mut Vec<Piece>, ch: char, style: Style) {
    match pieces.last_mut() {
        Some(last) if last.style == style => last.text.push(ch),
        _ => pieces.push(Piece::new(ch.to_string(), style)),
    }
}

fn push_piece(pieces: &mut Vec<Piece>, piece: Piece) {
    match pieces.last_mut() {
        Some(last) if last.style == piece.style => last.text.push_str(&piece.text),
        _ => pieces.push(piece),
    }
}

struct Wrapper {
    width: usize,
    lines: Vec<Vec<Piece>>,
    line: Vec<Piece>,
    line_width: usize,
    /// Style of the space owed before the next word, if any.
    space: Option<Style>,
    word: Vec<Piece>,
    word_width: usize,
}

impl Wrapper {
    fn new_line(&mut self) {
        self.lines.push(std::mem::take(&mut self.line));
        self.line_width = 0;
        self.space = None;
    }

    fn flush_word(&mut self) {
        if self.word.is_empty() {
            return;
        }
        let word = std::mem::take(&mut self.word);
        let word_width = std::mem::take(&mut self.word_width);
        let space = if self.line_width > 0 {
            self.space.take()
        } else {
            None
        };
        let gap = usize::from(space.is_some());
        let mut gap_used = 0;
        if self.line_width + gap + word_width <= self.width {
            if let Some(style) = space {
                push_piece(&mut self.line, Piece::new(" ", style));
                gap_used = 1;
            }
        } else if word_width <= self.width {
            self.new_line();
        } else {
            // Longer than a whole line: start on a fresh line and split anywhere.
            if self.line_width > 0 {
                self.new_line();
            }
            for piece in word {
                for ch in piece.text.chars() {
                    let w = ch.width().unwrap_or(0);
                    if self.line_width + w > self.width && self.line_width > 0 {
                        self.new_line();
                    }
                    push_char(&mut self.line, ch, piece.style);
                    self.line_width += w;
                }
            }
            self.space = None;
            return;
        }
        for piece in word {
            push_piece(&mut self.line, piece);
        }
        self.line_width += gap_used + word_width;
        self.space = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(width: usize, colors: bool, utf8: bool) -> Options {
        Options {
            width,
            colors,
            depth: ColorDepth::TrueColor,
            utf8,
            hyperlinks: colors,
            light: false,
            tab_width: 4,
        }
    }

    fn md(text: &str, width: usize) -> String {
        let mut none = |_: &str, _: &str| None;
        String::from_utf8(render(text, &opts(width, false, true), &mut none)).unwrap()
    }

    fn md_with(text: &str, opts: &Options) -> String {
        let mut none = |_: &str, _: &str| None;
        String::from_utf8(render(text, opts, &mut none)).unwrap()
    }

    fn md_images(text: &str, drawn: &Drawn, seen: &mut Vec<(String, u16)>) -> String {
        let mut none = |_: &str, _: &str| None;
        let mut images = |url: &str, cols: u16| {
            seen.push((url.to_string(), cols));
            (url != "skip.png").then(|| drawn.clone())
        };
        let out = render_with_images(text, &opts(40, false, true), &mut none, &mut images);
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn images_become_blocks_with_container_prefixes() {
        let drawn = Drawn {
            data: b"AB\nCD\n".to_vec(),
            cols: 2,
            rows: 2,
            method: crate::image::Method::HalfBlocks,
        };
        let mut seen = Vec::new();
        let out = md_images(
            "Text ![a](a.png) more\n\n- item\n\n  ![b](b.png)\n\n# H ![c](c.png)\n\n![d](skip.png)\n",
            &drawn,
            &mut seen,
        );
        assert_eq!(
            out,
            "Text\nAB\nCD\nmore\n\n• item\n\n  AB\n  CD\n\nH [image: c]\n\
             ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n\n[image: d]\n"
        );
        // Headings never ask; list items get their indent taken off the width.
        assert_eq!(
            seen,
            [
                ("a.png".to_string(), 40),
                ("b.png".to_string(), 38),
                ("skip.png".to_string(), 40)
            ]
        );
    }

    #[test]
    fn pixel_images_get_one_prefix_and_a_newline() {
        let drawn = Drawn {
            data: b"\x1bPq-\x1b\\".to_vec(),
            cols: 4,
            rows: 2,
            method: crate::image::Method::Sixel,
        };
        let out = md_images("> ![a](a.png)\n", &drawn, &mut Vec::new());
        assert_eq!(out, "│ \x1bPq-\x1b\\\n");
    }

    #[test]
    fn image_paths_stay_local() {
        let dir = std::env::temp_dir().join(format!("scat-md-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("sub dir")).unwrap();
        std::fs::write(dir.join("sub dir/a.png"), b"x").unwrap();
        let found = Some(dir.join("sub dir/a.png"));
        assert_eq!(local_image_path(&dir, "sub dir/a.png"), found);
        assert_eq!(local_image_path(&dir, "sub%20dir/a.png"), found);
        assert_eq!(local_image_path(&dir, "sub dir/a.png?raw=true#x"), found);
        let abs = dir.join("sub dir/a.png");
        assert_eq!(
            local_image_path(Path::new("/nonexistent"), abs.to_str().unwrap()),
            found
        );
        for remote in [
            "https://example.com/a.png",
            "http:sub dir/a.png",
            "file:sub dir/a.png",
            "data:image/png;base64,AAAA",
            "//example.com/a.png",
            "",
            "sub dir",
            "missing.png",
        ] {
            assert_eq!(local_image_path(&dir, remote), None, "{remote}");
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn wraps_paragraphs() {
        let out = md("one two three four five six seven eight nine ten", 20);
        assert_eq!(out, "one two three four\nfive six seven eight\nnine ten\n");
        for line in out.lines() {
            assert!(line.width() <= 20);
        }
    }

    #[test]
    fn splits_words_longer_than_a_line() {
        assert_eq!(md("a abcdefghijklmnop", 10), "a\nabcdefghij\nklmnop\n");
    }

    #[test]
    fn hanging_indent_in_lists() {
        let out = md("- alpha beta gamma delta epsilon\n- two\n", 20);
        assert_eq!(out, "• alpha beta gamma\n  delta epsilon\n• two\n");
        let out = md("1. alpha beta gamma delta epsilon\n2. two\n", 20);
        assert_eq!(out, "1. alpha beta gamma\n   delta epsilon\n2. two\n");
    }

    #[test]
    fn nested_lists_and_tasks() {
        let out = md(
            "- a\n  - b\n    - c\n- d\n\ntext\n\n- [ ] todo\n- [x] done\n",
            40,
        );
        assert_eq!(out, "• a\n  ◦ b\n    ▪ c\n• d\n\ntext\n\n☐ todo\n☑ done\n");
        let ascii = md_with("- a\n  - b\n- [x] c\n", &opts(40, false, false));
        assert_eq!(ascii, "* a\n  - b\n[x] c\n");
    }

    #[test]
    fn loose_lists_keep_blank_lines() {
        let out = md("Intro.\n\n- one\n\n- two\n\nAfter.\n", 40);
        assert_eq!(out, "Intro.\n\n• one\n\n• two\n\nAfter.\n");
    }

    #[test]
    fn headings_and_blocks() {
        let out = md(
            "# Title\n\ntext\n\n### Sub\n\n> quoted\n> more\n\n---\n",
            10,
        );
        assert_eq!(
            out,
            "Title\n━━━━━━━━━━\n\ntext\n\n### Sub\n\n│ quoted\n│ more\n\n──────────\n"
        );
        let ascii = md_with("# T\n\n> q\n>\n> > r\n", &opts(10, false, false));
        assert_eq!(ascii, "T\n==========\n\n| q\n|\n| | r\n");
    }

    #[test]
    fn code_blocks_are_indented_not_wrapped() {
        let out = md(
            "```rust\nfn main() { a_very_long_line_that_overflows(); }\n```\n",
            20,
        );
        assert_eq!(
            out,
            "    fn main() { a_very_long_line_that_overflows(); }\n"
        );
    }

    #[test]
    fn code_blocks_use_the_highlighter() {
        let mut seen = Vec::new();
        let mut hl = |lang: &str, code: &str| {
            seen.push((lang.to_string(), code.to_string()));
            Some(vec![b"\x1b[31mX\x1b[0m".to_vec()])
        };
        let out = render(
            "```rust title\nlet x;\n```\n",
            &opts(40, true, true),
            &mut hl,
        );
        assert_eq!(out, b"    \x1b[31mX\x1b[0m\n");
        assert_eq!(seen, [("rust".into(), "let x;\n".into())]);
    }

    #[test]
    fn tables_align_and_fit() {
        let out = md(
            "| a | b | c |\n|:--|:-:|--:|\n| x | yy | 1 |\n| long | z | 100 |\n",
            40,
        );
        assert_eq!(
            out,
            "┌──────┬────┬─────┐\n\
             │ a    │ b  │   c │\n\
             ├──────┼────┼─────┤\n\
             │ x    │ yy │   1 │\n\
             │ long │ z  │ 100 │\n\
             └──────┴────┴─────┘\n"
        );
        let narrow = md(
            "| key | description |\n|---|---|\n| a | one two three four five six |\n",
            24,
        );
        for line in narrow.lines() {
            assert!(line.width() <= 24, "{line:?} too wide");
        }
        assert!(
            narrow.contains("│ a   │ one two three  │\n│     │ four five six  │\n"),
            "{narrow}"
        );
    }

    #[test]
    fn links_show_urls_and_osc8() {
        let out = md("[docs](<https://x.y/a b>) and <https://x.y>", 80);
        assert_eq!(out, "docs (https://x.y/a b) and https://x.y\n");
        let colored = md_with("[docs](<https://x.y/a b>)", &opts(80, true, true));
        assert!(
            colored.contains("\x1b]8;;https://x.y/a%20b\x1b\\"),
            "{colored:?}"
        );
        assert!(colored.contains("\x1b]8;;\x1b\\"));
        let no_links = md_with(
            "[docs](https://x.y)",
            &Options {
                hyperlinks: false,
                ..opts(80, true, true)
            },
        );
        assert!(!no_links.contains("\x1b]8"));
    }

    #[test]
    fn hyperlink_targets_are_encoded() {
        assert_eq!(
            hyperlink_target("a b\x1b]0;x\x07é"),
            "a%20b%1B]0;x%07%C3%A9"
        );
    }

    #[test]
    fn document_escapes_never_pass() {
        let doc = "# T\x1b]0;evil\x07\n\ntext \x1b]0;evil\x07 `c\x1b[2J`\n\n```\nx\x1b]0;evil\x07\n```\n\n[l](u\x1b]0;evil\x07)\n\n<b>\x1b]0;evil\x07</b>\n";
        for colors in [false, true] {
            let out = md_with(doc, &opts(80, colors, true));
            assert!(!out.contains("\x1b]0;"), "{out:?}");
            assert!(!out.contains('\x07'), "{out:?}");
            assert!(!out.contains("\x1b[2J"), "{out:?}");
            assert!(out.contains("^[]0;evil^G"));
        }
    }

    #[test]
    fn colours_off_means_no_escapes() {
        let out = md(
            "# H\n\n*a* **b** `c` ~~d~~ [e](f) ![g](h)\n\n> [!NOTE]\n> n\n",
            80,
        );
        assert!(!out.contains('\x1b'));
        assert!(out.contains("a b `c` d e (f) [image: g]"), "{out}");
        assert!(out.contains("│ Note\n│ n\n"), "{out}");
    }

    #[test]
    fn colours_on_styles_text() {
        let out = md_with("**b** `c`", &opts(80, true, true));
        assert!(out.contains("\x1b[0;1mb"), "{out:?}");
        assert!(out.contains("48;5;236mc"), "{out:?}");
        assert!(out.ends_with("\x1b[0m\n"));
    }

    #[test]
    fn footnotes_go_last() {
        let out = md("Text[^1].\n\n[^1]: The note.\n\nMore.\n", 40);
        assert_eq!(
            out,
            "Text[1].\n\nMore.\n\n────────────────────\n[1] The note.\n"
        );
    }

    #[test]
    fn html_is_dimmed_and_comments_hidden() {
        let out = md(
            "<!-- hidden -->\n\n<div>\nshown\n</div>\n\nText<br>next\n",
            40,
        );
        assert_eq!(out, "<div>\nshown\n</div>\n\nText\nnext\n");
    }

    #[test]
    fn detects_markdown_files() {
        assert!(is_markdown(Some("README.md"), None));
        assert!(is_markdown(Some("notes.MARKDOWN"), None));
        assert!(!is_markdown(Some("md"), None));
        assert!(!is_markdown(Some(".md"), None));
        assert!(!is_markdown(Some("main.rs"), None));
        assert!(is_markdown(None, Some("md")));
        assert!(!is_markdown(Some("README.md"), Some("text")));
    }
}
