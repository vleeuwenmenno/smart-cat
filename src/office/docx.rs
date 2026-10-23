//! Word documents (docx): paragraphs, headings, lists, tables and run formatting.

use std::collections::HashMap;
use std::io::{Read, Seek};

use quick_xml::events::Event;

use super::md::{Inline, Md, Style, TableBuilder};
use super::xml::{self, attr, local, rel_id, text_of};
use super::{Converted, read_part, require_part};

pub fn convert<R: Read + Seek>(mut zip: zip::ZipArchive<R>) -> Result<Converted, String> {
    let document = require_part(&mut zip, "word/document.xml")?;
    let styles = read_part(&mut zip, "word/styles.xml")?
        .map(|d| style_names(&d))
        .unwrap_or_default();
    let numbering = read_part(&mut zip, "word/numbering.xml")?
        .map(|d| Numbering::parse(&d))
        .unwrap_or_default();
    let rels = read_part(&mut zip, "word/_rels/document.xml.rels")?
        .map(|d| relationships(&d))
        .unwrap_or_default();
    let pages = read_part(&mut zip, "docProps/app.xml")
        .ok()
        .flatten()
        .and_then(|d| xml::element_text(&d, "Pages"))
        .and_then(|p| p.trim().parse::<u32>().ok())
        .filter(|&p| p > 0);

    let markdown = Walker::new(&styles, &numbering, &rels).run(&document)?;
    let counts = match pages {
        Some(1) => "1 page".to_string(),
        Some(n) => format!("{n} pages"),
        None => String::new(),
    };
    Ok(Converted { markdown, counts })
}

/// `styleId` to style name from styles.xml.
fn style_names(data: &[u8]) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let mut reader = xml::reader(data);
    let mut current: Option<String> = None;
    while let Ok(event) = reader.read_event() {
        match event {
            Event::Start(e) if e.local_name().as_ref() == b"style" => {
                current = attr(&e, "styleId");
            }
            Event::Start(e) | Event::Empty(e) if e.local_name().as_ref() == b"name" => {
                if let (Some(id), Some(name)) = (&current, attr(&e, "val")) {
                    map.insert(id.clone(), name);
                }
            }
            Event::End(e) if e.local_name().as_ref() == b"style" => current = None,
            Event::Eof => break,
            _ => {}
        }
    }
    map
}

/// Relationship id to target, for hyperlinks.
fn relationships(data: &[u8]) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let mut reader = xml::reader(data);
    while let Ok(event) = reader.read_event() {
        match event {
            Event::Start(e) | Event::Empty(e) if e.local_name().as_ref() == b"Relationship" => {
                if let (Some(id), Some(target)) = (attr(&e, "Id"), attr(&e, "Target")) {
                    map.insert(id, target);
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    map
}

#[derive(Default)]
struct Numbering {
    /// numId to abstractNumId.
    nums: HashMap<String, String>,
    /// (abstractNumId, ilvl) to whether the level is numbered.
    levels: HashMap<(String, String), bool>,
}

impl Numbering {
    fn parse(data: &[u8]) -> Self {
        let mut out = Self::default();
        let mut reader = xml::reader(data);
        let mut abstract_id: Option<String> = None;
        let mut level: Option<String> = None;
        let mut num_id: Option<String> = None;
        while let Ok(event) = reader.read_event() {
            match event {
                Event::Start(e) => match e.local_name().as_ref() {
                    b"abstractNum" => abstract_id = attr(&e, "abstractNumId"),
                    b"lvl" => level = attr(&e, "ilvl"),
                    b"num" => num_id = attr(&e, "numId"),
                    b"numFmt" => out.set_level(&abstract_id, &level, &e),
                    _ => {}
                },
                Event::Empty(e) => match e.local_name().as_ref() {
                    b"numFmt" => out.set_level(&abstract_id, &level, &e),
                    b"abstractNumId" => {
                        if let (Some(num), Some(abs)) = (&num_id, attr(&e, "val")) {
                            out.nums.insert(num.clone(), abs);
                        }
                    }
                    _ => {}
                },
                Event::End(e) => match e.local_name().as_ref() {
                    b"abstractNum" => abstract_id = None,
                    b"lvl" => level = None,
                    b"num" => num_id = None,
                    _ => {}
                },
                Event::Eof => break,
                _ => {}
            }
        }
        out
    }

    fn set_level(
        &mut self,
        abstract_id: &Option<String>,
        level: &Option<String>,
        e: &quick_xml::events::BytesStart,
    ) {
        if let (Some(a), Some(l), Some(fmt)) = (abstract_id, level, attr(e, "val")) {
            self.levels.insert(
                (a.clone(), l.clone()),
                !matches!(fmt.as_str(), "bullet" | "none"),
            );
        }
    }

    fn ordered(&self, num_id: &str, level: &str) -> bool {
        self.nums
            .get(num_id)
            .and_then(|abs| self.levels.get(&(abs.clone(), level.to_string())))
            .copied()
            .unwrap_or(false)
    }
}

/// Heading level for a paragraph style name or id.
fn heading_level(id: &str, name: Option<&str>) -> Option<usize> {
    let name = name.unwrap_or(id).to_ascii_lowercase().replace(' ', "");
    if name == "title" {
        return Some(1);
    }
    let rest = name.strip_prefix("heading")?;
    let level: usize = rest.parse().ok()?;
    (1..=9).contains(&level).then_some(level.min(6))
}

#[derive(Default)]
struct Paragraph {
    style: Option<String>,
    num_id: Option<String>,
    level: Option<String>,
    inline: Inline,
}

struct Walker<'a> {
    styles: &'a HashMap<String, String>,
    numbering: &'a Numbering,
    rels: &'a HashMap<String, String>,
    md: Md,
}

impl<'a> Walker<'a> {
    fn new(
        styles: &'a HashMap<String, String>,
        numbering: &'a Numbering,
        rels: &'a HashMap<String, String>,
    ) -> Self {
        Self {
            styles,
            numbering,
            rels,
            md: Md::new(),
        }
    }

    fn run(mut self, document: &[u8]) -> Result<String, String> {
        let mut reader = xml::reader(document);
        let mut para: Option<Paragraph> = None;
        let mut in_run = false;
        let mut style = Style::default();
        let mut in_text = false;
        let mut link: Option<String> = None;
        let mut table_depth = 0usize;
        let mut table = TableBuilder::default();
        // Text boxes and fallbacks hold paragraphs of their own or copies: skipped.
        let mut skip: Option<(Vec<u8>, usize)> = None;
        loop {
            if self.md.is_full() {
                break;
            }
            let event = reader
                .read_event()
                .map_err(|err| format!("word/document.xml: {err}"))?;
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
                        b"p" if is_start => para = Some(Paragraph::default()),
                        b"pStyle" => {
                            if let Some(p) = &mut para {
                                p.style = attr(e, "val");
                            }
                        }
                        b"numId" => {
                            if let Some(p) = &mut para {
                                p.num_id = attr(e, "val");
                            }
                        }
                        b"ilvl" => {
                            if let Some(p) = &mut para {
                                p.level = attr(e, "val");
                            }
                        }
                        b"r" if is_start => {
                            in_run = true;
                            style = Style::default();
                        }
                        b"b" | b"bCs" if in_run => style.bold = on(e),
                        b"i" | b"iCs" if in_run => style.italic = on(e),
                        b"strike" | b"dstrike" if in_run => style.strike = on(e),
                        b"rStyle" if in_run => match attr(e, "val").as_deref() {
                            Some("Strong") => style.bold = true,
                            Some("Emphasis") => style.italic = true,
                            _ => {}
                        },
                        b"t" if is_start => in_text = true,
                        b"tab" if in_run => push(&mut para, " ", style, link.as_deref()),
                        b"br" | b"cr" if in_run => {
                            if let Some(p) = &mut para
                                && attr(e, "type").as_deref() != Some("page")
                            {
                                p.inline.line_break();
                            }
                        }
                        b"drawing" | b"pict" if is_start => {
                            push(&mut para, "[image]", Style::default(), None);
                        }
                        b"hyperlink" if is_start => {
                            link = rel_id(e).and_then(|id| self.rels.get(&id).cloned());
                        }
                        b"tbl" if is_start => {
                            table_depth += 1;
                            if table_depth == 1 {
                                table = TableBuilder::default();
                            }
                        }
                        b"tr" if is_start && table_depth == 1 => table.start_row(),
                        b"tc" if is_start && table_depth == 1 => table.start_cell(),
                        b"txbxContent" | b"Fallback" if is_start => {
                            skip = Some((name.clone(), 0));
                        }
                        _ => {}
                    }
                }
                Event::End(e) => match e.local_name().as_ref() {
                    b"t" => in_text = false,
                    b"r" => in_run = false,
                    b"hyperlink" => link = None,
                    b"p" => {
                        if let Some(p) = para.take() {
                            self.end_paragraph(p, table_depth, &mut table);
                        }
                    }
                    b"tc" if table_depth == 1 => table.end_cell(),
                    b"tr" if table_depth == 1 => table.end_row(),
                    b"tbl" => {
                        table_depth = table_depth.saturating_sub(1);
                        if table_depth == 0 {
                            let rows = std::mem::take(&mut table).finish();
                            self.md.table(&rows);
                        }
                    }
                    _ => {}
                },
                ev if in_text => {
                    if let Some(text) = text_of(ev) {
                        push(&mut para, &text, style, link.as_deref());
                    }
                }
                _ => {}
            }
        }
        Ok(self.md.finish())
    }

    fn end_paragraph(&mut self, p: Paragraph, table_depth: usize, table: &mut TableBuilder) {
        let blank = p.inline.is_blank();
        let text = p.inline.finish();
        if table_depth > 0 {
            table.add(&text);
            return;
        }
        if blank {
            return;
        }
        let style_id = p.style.as_deref().unwrap_or("");
        let style_name = self.styles.get(style_id).map(String::as_str);
        if let Some(level) = heading_level(style_id, style_name) {
            self.md.heading(level, &text);
            return;
        }
        let lowered = style_name.unwrap_or(style_id).to_ascii_lowercase();
        let level = p
            .level
            .as_deref()
            .and_then(|l| l.parse::<usize>().ok())
            .unwrap_or(0);
        match p.num_id.as_deref() {
            // numId 0 switches numbering off.
            Some(id) if id != "0" => {
                let ordered = self
                    .numbering
                    .ordered(id, p.level.as_deref().unwrap_or("0"));
                self.md.list_item(level, ordered, &text);
            }
            _ if lowered.starts_with("listbullet") || lowered.starts_with("list bullet") => {
                self.md.list_item(level, false, &text);
            }
            _ if lowered.starts_with("listnumber") || lowered.starts_with("list number") => {
                self.md.list_item(level, true, &text);
            }
            _ => self.md.paragraph(&text),
        }
    }
}

fn push(para: &mut Option<Paragraph>, text: &str, style: Style, link: Option<&str>) {
    if let Some(p) = para {
        p.inline.push(text, style, link);
    }
}

/// A toggle property: present without a value, or with a value other than off.
fn on(e: &quick_xml::events::BytesStart) -> bool {
    !matches!(
        attr(e, "val").as_deref(),
        Some("0" | "false" | "off" | "none")
    )
}
