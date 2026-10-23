//! OpenDocument text and presentations (odt, odp).

use std::collections::HashMap;
use std::io::{Read, Seek};

use quick_xml::events::Event;

use super::md::{Inline, Md, Style, TableBuilder};
use super::xml::{self, attr, local, text_of};
use super::{Converted, Format, MAX_COLUMNS, MAX_SLIDES, read_part, require_part};

pub fn convert<R: Read + Seek>(
    mut zip: zip::ZipArchive<R>,
    format: Format,
) -> Result<Converted, String> {
    if let Some(mimetype) = read_part(&mut zip, "mimetype")?
        && !mimetype.starts_with(b"application/vnd.oasis.opendocument.")
    {
        return Err("mimetype is not an OpenDocument type".into());
    }
    let content = require_part(&mut zip, "content.xml")?;
    let mut styles = Styles::default();
    if let Some(data) = read_part(&mut zip, "styles.xml")? {
        styles.parse(&data);
    }
    styles.parse(&content);

    let mut walker = Walker {
        styles: &styles,
        md: Md::new(),
        slides: 0,
        presentation: format == Format::Odp,
    };
    walker.run(&content)?;
    let slides = walker.slides;
    let markdown = walker.md.finish();

    let counts = if format == Format::Odp {
        match slides {
            1 => "1 slide".to_string(),
            n => format!("{n} slides"),
        }
    } else {
        read_part(&mut zip, "meta.xml")
            .ok()
            .flatten()
            .and_then(|d| page_count(&d))
            .map(|n| {
                if n == 1 {
                    "1 page".into()
                } else {
                    format!("{n} pages")
                }
            })
            .unwrap_or_default()
    };
    Ok(Converted { markdown, counts })
}

fn page_count(meta: &[u8]) -> Option<u32> {
    let mut reader = xml::reader(meta);
    while let Ok(event) = reader.read_event() {
        match event {
            Event::Start(e) | Event::Empty(e)
                if e.local_name().as_ref() == b"document-statistic" =>
            {
                return attr(&e, "page-count")?.parse().ok().filter(|&n| n > 0);
            }
            Event::Eof => break,
            _ => {}
        }
    }
    None
}

#[derive(Default)]
struct Styles {
    text: HashMap<String, Style>,
    /// List style name to whether its first level is numbered.
    lists: HashMap<String, bool>,
}

impl Styles {
    fn parse(&mut self, data: &[u8]) {
        let mut reader = xml::reader(data);
        let mut style: Option<String> = None;
        let mut list: Option<String> = None;
        while let Ok(event) = reader.read_event() {
            match event {
                Event::Start(e) | Event::Empty(e) => match e.local_name().as_ref() {
                    b"style" => style = attr(&e, "name"),
                    b"list-style" => list = attr(&e, "name"),
                    b"text-properties" => {
                        if let Some(name) = &style {
                            let entry = self.text.entry(name.clone()).or_default();
                            entry.bold |= attr(&e, "font-weight").is_some_and(|w| {
                                w == "bold" || w.parse::<u32>().is_ok_and(|n| n >= 600)
                            });
                            entry.italic |= attr(&e, "font-style")
                                .is_some_and(|s| s == "italic" || s == "oblique");
                            entry.strike |=
                                attr(&e, "text-line-through-type").is_some_and(|s| s != "none");
                        }
                    }
                    b"list-level-style-number" => {
                        if let Some(name) = &list {
                            self.lists.entry(name.clone()).or_insert(true);
                        }
                    }
                    b"list-level-style-bullet" | b"list-level-style-image" => {
                        if let Some(name) = &list {
                            self.lists.entry(name.clone()).or_insert(false);
                        }
                    }
                    _ => {}
                },
                Event::End(e) => match e.local_name().as_ref() {
                    b"style" => style = None,
                    b"list-style" => list = None,
                    _ => {}
                },
                Event::Eof => break,
                _ => {}
            }
        }
    }

    fn style(&self, name: Option<&str>) -> Style {
        name.and_then(|n| self.text.get(n))
            .copied()
            .unwrap_or_default()
    }
}

#[derive(PartialEq, Eq, Clone, Copy)]
enum Role {
    Normal,
    Title,
    Hidden,
}

struct Block {
    heading: Option<usize>,
    inline: Inline,
    base: Style,
}

struct Walker<'a> {
    styles: &'a Styles,
    md: Md,
    slides: usize,
    presentation: bool,
}

/// Collapses runs of white space to one space, as ODF does.
fn collapse(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut space = false;
    for c in text.chars() {
        if c.is_whitespace() {
            if !space {
                out.push(' ');
            }
            space = true;
        } else {
            out.push(c);
            space = false;
        }
    }
    out
}

impl Walker<'_> {
    fn run(&mut self, content: &[u8]) -> Result<(), String> {
        let mut reader = xml::reader(content);
        let mut block: Option<Block> = None;
        let mut spans: Vec<Style> = Vec::new();
        let mut link: Option<String> = None;
        let mut lists: Vec<bool> = Vec::new();
        let mut frames: Vec<Role> = Vec::new();
        let mut table_depth = 0usize;
        let mut table = TableBuilder::default();
        let mut cell_repeat = 1usize;
        let mut skip: Option<(Vec<u8>, usize)> = None;
        loop {
            if self.md.is_full() {
                break;
            }
            let event = reader
                .read_event()
                .map_err(|err| format!("content.xml: {err}"))?;
            if let Some((name, depth)) = &mut skip {
                match &event {
                    Event::Start(e) if e.local_name().as_ref() == name.as_slice() => *depth += 1,
                    Event::End(e) if e.local_name().as_ref() == name.as_slice() => {
                        if *depth == 0 {
                            skip = None;
                        } else {
                            *depth -= 1;
                        }
                    }
                    Event::Eof => break,
                    _ => {}
                }
                continue;
            }
            match &event {
                Event::Eof => break,
                Event::Start(e) | Event::Empty(e) => {
                    let is_start = matches!(event, Event::Start(_));
                    let name = local(e);
                    match name.as_slice() {
                        b"annotation" | b"tracked-changes" | b"notes" | b"note" if is_start => {
                            skip = Some((name.clone(), 0));
                        }
                        b"page" if is_start && self.presentation => {
                            self.slides += 1;
                            if self.slides > MAX_SLIDES {
                                skip = Some((name.clone(), 0));
                                continue;
                            }
                            self.md.heading(2, &format!("Slide {}", self.slides));
                        }
                        b"frame" if is_start => {
                            frames.push(match attr(e, "class").as_deref() {
                                Some("title") => Role::Title,
                                Some("footer" | "date-time" | "header" | "page-number") => {
                                    Role::Hidden
                                }
                                _ => Role::Normal,
                            });
                        }
                        b"list" if is_start => {
                            let ordered = attr(e, "style-name")
                                .and_then(|n| self.styles.lists.get(&n).copied())
                                .or_else(|| lists.last().copied())
                                .unwrap_or(false);
                            lists.push(ordered);
                        }
                        b"p" | b"h" if is_start => {
                            let heading = (name == b"h").then(|| {
                                attr(e, "outline-level")
                                    .and_then(|l| l.parse().ok())
                                    .unwrap_or(1)
                            });
                            // Heading styles are bold already: no markers inside `#`.
                            let base = match heading {
                                Some(_) => Style::default(),
                                None => self.styles.style(attr(e, "style-name").as_deref()),
                            };
                            block = Some(Block {
                                heading,
                                inline: Inline::default(),
                                base,
                            });
                        }
                        b"span" if is_start => {
                            let base = spans.last().copied().unwrap_or_default();
                            let own = self.styles.style(attr(e, "style-name").as_deref());
                            spans.push(Style {
                                bold: base.bold || own.bold,
                                italic: base.italic || own.italic,
                                strike: base.strike || own.strike,
                            });
                        }
                        b"a" if is_start => link = attr(e, "href"),
                        b"s" => {
                            let count = attr(e, "c").and_then(|c| c.parse().ok()).unwrap_or(1);
                            push(
                                &mut block,
                                &" ".repeat(usize::min(count, 64)),
                                &spans,
                                &link,
                            );
                        }
                        b"tab" => push(&mut block, " ", &spans, &link),
                        b"line-break" => {
                            if let Some(b) = &mut block {
                                b.inline.line_break();
                            }
                        }
                        b"image" if table_depth == 0 || block.is_some() => {
                            if block.is_some() {
                                push(&mut block, "[image]", &[], &None);
                            } else {
                                self.md.paragraph("[image]");
                            }
                        }
                        b"table" if is_start => {
                            table_depth += 1;
                            if table_depth == 1 {
                                table = TableBuilder::default();
                            }
                        }
                        b"table-row" if is_start && table_depth == 1 => table.start_row(),
                        b"table-cell" if table_depth == 1 => {
                            cell_repeat = attr(e, "number-columns-repeated")
                                .and_then(|n| n.parse().ok())
                                .unwrap_or(1);
                            table.start_cell();
                            if !is_start {
                                table.end_cell_repeated(cell_repeat.min(MAX_COLUMNS));
                            }
                        }
                        b"covered-table-cell" if table_depth == 1 => {
                            table.start_cell();
                            table.end_cell_repeated(1);
                        }
                        _ => {}
                    }
                }
                Event::End(e) => match e.local_name().as_ref() {
                    b"frame" => {
                        frames.pop();
                    }
                    b"list" => {
                        lists.pop();
                    }
                    b"span" => {
                        spans.pop();
                    }
                    b"a" => link = None,
                    b"p" | b"h" => {
                        if let Some(b) = block.take() {
                            let role = if frames.contains(&Role::Hidden) {
                                Role::Hidden
                            } else if frames.contains(&Role::Title) {
                                Role::Title
                            } else {
                                Role::Normal
                            };
                            self.end_block(b, role, &lists, table_depth, &mut table);
                        }
                    }
                    b"table-cell" if table_depth == 1 => {
                        table.end_cell_repeated(cell_repeat.min(MAX_COLUMNS));
                    }
                    b"table-row" if table_depth == 1 => table.end_row(),
                    b"table" => {
                        table_depth = table_depth.saturating_sub(1);
                        if table_depth == 0 {
                            self.md.table(&std::mem::take(&mut table).finish());
                        }
                    }
                    _ => {}
                },
                ev if block.is_some() => {
                    if let Some(text) = text_of(ev) {
                        push(&mut block, &collapse(&text), &spans, &link);
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn end_block(
        &mut self,
        b: Block,
        role: Role,
        lists: &[bool],
        table_depth: usize,
        table: &mut TableBuilder,
    ) {
        let text = b.inline.finish();
        if table_depth > 0 {
            table.add(&text);
            return;
        }
        if text.is_empty() || role == Role::Hidden {
            return;
        }
        if let Some(level) = b.heading {
            self.md.heading(level, &text);
        } else if role == Role::Title {
            self.md.heading(3, &text);
        } else if let Some(&ordered) = lists.last() {
            self.md.list_item(lists.len() - 1, ordered, &text);
        } else {
            self.md.paragraph(&text);
        }
    }
}

fn push(block: &mut Option<Block>, text: &str, spans: &[Style], link: &Option<String>) {
    if let Some(b) = block {
        let span = spans.last().copied().unwrap_or_default();
        let style = Style {
            bold: span.bold || b.base.bold,
            italic: span.italic || b.base.italic,
            strike: span.strike || b.base.strike,
        };
        b.inline.push(text, style, link.as_deref());
    }
}
