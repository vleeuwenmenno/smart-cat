//! PowerPoint presentations (pptx): one `## Slide N` section per slide.

use std::collections::HashMap;
use std::io::{Read, Seek};

use quick_xml::events::Event;

use super::md::{Inline, Md, Style, TableBuilder};
use super::xml::{self, attr, local, rel_id, text_of};
use super::{Converted, MAX_SLIDES, read_part, require_part};

pub fn convert<R: Read + Seek>(mut zip: zip::ZipArchive<R>) -> Result<Converted, String> {
    let presentation = require_part(&mut zip, "ppt/presentation.xml")?;
    let rels = read_part(&mut zip, "ppt/_rels/presentation.xml.rels")?
        .map(|d| relationships(&d))
        .unwrap_or_default();
    let mut slides = slide_paths(&presentation, &rels);
    if slides.is_empty() {
        slides = fallback_slide_paths(&zip);
    }

    let mut md = Md::new();
    for (i, path) in slides.iter().take(MAX_SLIDES).enumerate() {
        if md.is_full() {
            break;
        }
        md.heading(2, &format!("Slide {}", i + 1));
        let data = require_part(&mut zip, path)?;
        let rels_path = match path.rsplit_once('/') {
            Some((dir, file)) => format!("{dir}/_rels/{file}.rels"),
            None => format!("_rels/{path}.rels"),
        };
        let slide_rels = read_part(&mut zip, &rels_path)?
            .map(|d| relationships(&d))
            .unwrap_or_default();
        walk_slide(&data, &slide_rels, &mut md).map_err(|err| format!("{path}: {err}"))?;
    }
    if slides.len() > MAX_SLIDES {
        md.paragraph(&format!("*… {} more slides*", slides.len() - MAX_SLIDES));
    }
    let counts = match slides.len() {
        1 => "1 slide".to_string(),
        n => format!("{n} slides"),
    };
    Ok(Converted {
        markdown: md.finish(),
        counts,
    })
}

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

/// Slide part names in presentation order.
fn slide_paths(presentation: &[u8], rels: &HashMap<String, String>) -> Vec<String> {
    let mut paths = Vec::new();
    let mut reader = xml::reader(presentation);
    while let Ok(event) = reader.read_event() {
        match event {
            Event::Start(e) | Event::Empty(e) if e.local_name().as_ref() == b"sldId" => {
                if let Some(target) = rel_id(&e).and_then(|id| rels.get(&id)) {
                    paths.push(match target.strip_prefix('/') {
                        Some(abs) => abs.to_string(),
                        None => format!("ppt/{target}"),
                    });
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    paths
}

/// Without a slide list: `ppt/slides/slideN.xml` in numeric order.
fn fallback_slide_paths<R: Read + Seek>(zip: &zip::ZipArchive<R>) -> Vec<String> {
    let mut found: Vec<(u32, String)> = zip
        .file_names()
        .filter_map(|name| {
            let number = name
                .strip_prefix("ppt/slides/slide")?
                .strip_suffix(".xml")?
                .parse()
                .ok()?;
            Some((number, name.to_string()))
        })
        .collect();
    found.sort();
    found.into_iter().map(|(_, name)| name).collect()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Role {
    Title,
    Body,
    Other,
    /// Date, footer, slide number: not shown.
    Hidden,
}

#[derive(Default)]
struct Paragraph {
    level: usize,
    bullet: Option<bool>,
    no_bullet: bool,
    inline: Inline,
}

fn walk_slide(data: &[u8], rels: &HashMap<String, String>, md: &mut Md) -> Result<(), String> {
    let mut reader = xml::reader(data);
    let mut role = Role::Other;
    let mut para: Option<Paragraph> = None;
    let mut style = Style::default();
    let mut link: Option<String> = None;
    let mut in_run_props = false;
    let mut in_text = false;
    let mut table_depth = 0usize;
    let mut table = TableBuilder::default();
    loop {
        if md.is_full() {
            break;
        }
        let event = reader.read_event().map_err(|err| err.to_string())?;
        match &event {
            Event::Eof => break,
            Event::Start(e) | Event::Empty(e) => {
                let is_start = matches!(event, Event::Start(_));
                match local(e).as_slice() {
                    b"sp" if is_start => role = Role::Other,
                    b"ph" => {
                        role = match attr(e, "type").as_deref() {
                            Some("title" | "ctrTitle") => Role::Title,
                            Some("dt" | "ftr" | "sldNum" | "hdr") => Role::Hidden,
                            Some("body") | None => Role::Body,
                            Some(_) => Role::Other,
                        }
                    }
                    b"p" if is_start => para = Some(Paragraph::default()),
                    b"pPr" => {
                        if let Some(p) = &mut para {
                            p.level = attr(e, "lvl").and_then(|l| l.parse().ok()).unwrap_or(0);
                        }
                    }
                    b"buChar" => set_bullet(&mut para, false),
                    b"buAutoNum" => set_bullet(&mut para, true),
                    b"buNone" => {
                        if let Some(p) = &mut para {
                            p.no_bullet = true;
                        }
                    }
                    b"r" if is_start => {
                        style = Style::default();
                        link = None;
                    }
                    b"rPr" => {
                        in_run_props = is_start;
                        style.bold = attr(e, "b").is_some_and(|v| v == "1" || v == "true");
                        style.italic = attr(e, "i").is_some_and(|v| v == "1" || v == "true");
                        style.strike = attr(e, "strike").is_some_and(|v| v.starts_with("sng"));
                    }
                    b"hlinkClick" => {
                        link = rel_id(e).and_then(|id| rels.get(&id).cloned());
                    }
                    b"t" if is_start => in_text = true,
                    b"br" => {
                        if let Some(p) = &mut para {
                            p.inline.line_break();
                        }
                    }
                    b"pic" if is_start && table_depth == 0 => md.paragraph("[image]"),
                    b"tbl" if is_start => {
                        table_depth += 1;
                        if table_depth == 1 {
                            table = TableBuilder::default();
                        }
                    }
                    b"tr" if is_start && table_depth == 1 => table.start_row(),
                    b"tc" if is_start && table_depth == 1 => table.start_cell(),
                    _ => {}
                }
            }
            Event::End(e) => match e.local_name().as_ref() {
                b"t" => in_text = false,
                b"rPr" => in_run_props = false,
                b"p" => {
                    if let Some(p) = para.take() {
                        end_paragraph(p, role, table_depth, &mut table, md);
                    }
                }
                b"tc" if table_depth == 1 => table.end_cell(),
                b"tr" if table_depth == 1 => table.end_row(),
                b"tbl" => {
                    table_depth = table_depth.saturating_sub(1);
                    if table_depth == 0 {
                        md.table(&std::mem::take(&mut table).finish());
                    }
                }
                _ => {}
            },
            ev if in_text && !in_run_props => {
                if let (Some(text), Some(p)) = (text_of(ev), &mut para) {
                    p.inline.push(&text, style, link.as_deref());
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn set_bullet(para: &mut Option<Paragraph>, ordered: bool) {
    if let Some(p) = para {
        p.bullet = Some(ordered);
    }
}

fn end_paragraph(
    p: Paragraph,
    role: Role,
    table_depth: usize,
    table: &mut TableBuilder,
    md: &mut Md,
) {
    let blank = p.inline.is_blank();
    let text = p.inline.finish();
    if table_depth > 0 {
        table.add(&text);
        return;
    }
    if blank {
        return;
    }
    match role {
        Role::Hidden => {}
        Role::Title => md.heading(3, &text),
        Role::Body | Role::Other => {
            let default_bullet = role == Role::Body && !p.no_bullet;
            match p.bullet {
                Some(ordered) if !p.no_bullet => md.list_item(p.level, ordered, &text),
                None if default_bullet || p.level > 0 => md.list_item(p.level, false, &text),
                _ => md.paragraph(&text),
            }
        }
    }
}
