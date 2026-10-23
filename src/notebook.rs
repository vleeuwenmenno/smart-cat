//! Jupyter notebook view: a dim header, then every cell in order. Markdown cells go
//! through the markdown renderer, code cells are highlighted, and outputs follow their
//! cell: text, tracebacks, images and short placeholders.
//!
//! Everything in the file is untrusted. Text is cleaned by the caller's `clean` hook,
//! code goes through the sanitizer, and base64 payloads are size-capped before and
//! after decoding. Only nbformat 4 is read; anything else is an error and the caller
//! falls back to the JSON view.

use serde_json::Value;

use crate::image::Drawn;
use crate::render::sanitize_line;

/// Notebooks larger than this are not parsed.
pub const MAX_BYTES: u64 = 64 * 1024 * 1024;
/// One decoded image payload may not be larger than this.
const MAX_IMAGE_BYTES: usize = 16 * 1024 * 1024;
/// Output lines shown per output before `... N more lines`.
const MAX_OUTPUT_LINES: usize = 200;
/// Images are at most this many rows high.
pub const IMAGE_ROWS: u16 = 20;
/// Content of code cells and text outputs is indented by this.
const INDENT: &str = "  ";

const DIM: &[u8] = b"\x1b[2m";
const BOLD_RED: &[u8] = b"\x1b[1;31m";
const RESET: &[u8] = b"\x1b[0m";

pub struct Options {
    pub colors: bool,
    pub utf8: bool,
    pub width: usize,
    pub tab_width: usize,
    /// No header and no `In`/`Out` labels.
    pub plain: bool,
}

/// Draws an image for markdown: `(destination, max columns)`.
pub type MarkdownImages<'a> = &'a mut dyn FnMut(&str, u16) -> Option<Drawn>;

/// Encoded code lines, one per source line, without newlines.
pub type HighlightedLines = Vec<Vec<u8>>;

/// What the notebook view needs from the renderer.
pub struct Hooks<'a> {
    /// Renders markdown; the second argument draws an image for a destination.
    pub markdown: &'a mut dyn FnMut(&str, MarkdownImages) -> Vec<u8>,
    /// Highlights code: `(language, code)` to encoded lines without newlines.
    pub highlight: &'a mut dyn FnMut(&str, &str) -> Option<HighlightedLines>,
    /// Draws `(image bytes, is svg, max columns)`; `None` keeps a placeholder.
    pub image: &'a mut dyn FnMut(Vec<u8>, bool, u16) -> Option<Drawn>,
    /// Cleans one line of output text for the terminal (SGR kept per the `ansi` setting).
    pub clean: &'a mut dyn FnMut(&str) -> Vec<u8>,
}

/// Parses and validates a notebook.
pub fn parse(data: &[u8]) -> Result<Value, String> {
    let value: Value = serde_json::from_slice(data).map_err(|e| format!("invalid JSON: {e}"))?;
    let object = value.as_object().ok_or("not a JSON object")?;
    match object.get("nbformat").and_then(Value::as_u64) {
        Some(4) => {}
        Some(n) => return Err(format!("unsupported nbformat {n}")),
        None => return Err("missing nbformat".into()),
    }
    if !object.get("cells").is_some_and(Value::is_array) {
        return Err("missing cells".into());
    }
    Ok(value)
}

/// True when the file name or forced language says "notebook".
pub fn is_notebook(file_name: Option<&str>, language: Option<&str>) -> bool {
    if let Some(language) = language {
        return language.eq_ignore_ascii_case("ipynb") || language.eq_ignore_ascii_case("notebook");
    }
    file_name
        .and_then(|name| name.rsplit_once('.'))
        .is_some_and(|(stem, ext)| !stem.is_empty() && ext.eq_ignore_ascii_case("ipynb"))
}

/// Whether the notebook source mentions an image output (a cheap pre-scan).
pub fn may_draw_images(data: &[u8]) -> bool {
    [
        &b"image/png"[..],
        b"image/jpeg",
        b"image/svg+xml",
        b"attachment:",
    ]
    .iter()
    .any(|needle| data.windows(needle.len()).any(|w| w == *needle))
}

/// Renders a parsed notebook.
pub fn render(nb: &Value, opts: &Options, hooks: &mut Hooks) -> Vec<u8> {
    let mut out = Vec::new();
    let cells: &[Value] = nb["cells"].as_array().map_or(&[], Vec::as_slice);
    let language = language_of(nb);
    if !opts.plain {
        let header = header_text(nb, &language, cells.len(), opts.utf8);
        dim_line(&mut out, &header, opts);
        out.push(b'\n');
    }
    for (index, cell) in cells.iter().enumerate() {
        if index > 0 {
            out.push(b'\n');
        }
        match cell["cell_type"].as_str() {
            Some("markdown") => markdown_cell(&mut out, cell, opts, hooks),
            Some("code") => code_cell(&mut out, cell, &language, opts, hooks),
            _ => raw_cell(&mut out, cell, opts, hooks),
        }
    }
    out
}

fn language_of(nb: &Value) -> String {
    let meta = &nb["metadata"];
    meta["kernelspec"]["language"]
        .as_str()
        .or_else(|| meta["language_info"]["name"].as_str())
        .unwrap_or("python")
        .to_ascii_lowercase()
}

fn header_text(nb: &Value, language: &str, cells: usize, utf8: bool) -> String {
    let meta = &nb["metadata"];
    let name = meta["kernelspec"]["display_name"]
        .as_str()
        .or_else(|| meta["language_info"]["name"].as_str())
        .unwrap_or(language);
    let mut kernel = name.to_string();
    if let Some(version) = meta["language_info"]["version"].as_str()
        && !kernel.contains(version)
    {
        kernel.push(' ');
        kernel.push_str(version);
    }
    let sep = if utf8 { " · " } else { " - " };
    let noun = if cells == 1 { "cell" } else { "cells" };
    sanitize_line(
        &format!("Jupyter notebook{sep}{kernel}{sep}{cells} {noun}"),
        4,
    )
}

fn dim_line(out: &mut Vec<u8>, text: &str, opts: &Options) {
    if opts.colors {
        out.extend_from_slice(DIM);
    }
    out.extend_from_slice(text.as_bytes());
    if opts.colors {
        out.extend_from_slice(RESET);
    }
    out.push(b'\n');
}

/// A multi-line field: a string, or an array of strings to concatenate.
fn text_of(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Array(items) => items.iter().filter_map(Value::as_str).collect(),
        _ => String::new(),
    }
}

fn count_label(value: &Value) -> String {
    value.as_u64().map_or_else(|| " ".into(), |n| n.to_string())
}

fn max_cols(opts: &Options) -> u16 {
    u16::try_from(opts.width).unwrap_or(u16::MAX)
}

fn markdown_cell(out: &mut Vec<u8>, cell: &Value, opts: &Options, hooks: &mut Hooks) {
    let text = text_of(&cell["source"]);
    let attachments = &cell["attachments"];
    let Hooks {
        markdown, image, ..
    } = hooks;
    let cols = max_cols(opts);
    let mut attached = |dest: &str, _cols: u16| -> Option<Drawn> {
        let name = dest.strip_prefix("attachment:")?;
        let bundle = attachments.get(name)?.as_object()?;
        let (_, payload) = bundle
            .iter()
            .find(|(mime, _)| mime.starts_with("image/") && *mime != "image/svg+xml")?;
        let data = decode_base64(&text_of(payload), MAX_IMAGE_BYTES)?;
        image(data, false, cols)
    };
    out.extend_from_slice(&markdown(&text, &mut attached));
}

fn raw_cell(out: &mut Vec<u8>, cell: &Value, opts: &Options, hooks: &mut Hooks) {
    let text = text_of(&cell["source"]);
    for line in text.lines() {
        let line = (hooks.clean)(line);
        if opts.colors {
            out.extend_from_slice(DIM);
        }
        out.extend_from_slice(&line);
        if opts.colors {
            out.extend_from_slice(RESET);
        }
        out.push(b'\n');
    }
}

fn label(out: &mut Vec<u8>, text: &str, opts: &Options) {
    if !opts.plain {
        dim_line(out, text, opts);
    }
}

fn code_cell(out: &mut Vec<u8>, cell: &Value, language: &str, opts: &Options, hooks: &mut Hooks) {
    let source = text_of(&cell["source"]);
    label(
        out,
        &format!("In [{}]:", count_label(&cell["execution_count"])),
        opts,
    );
    let highlighted = if opts.colors {
        (hooks.highlight)(language, &source)
    } else {
        None
    };
    match highlighted {
        Some(lines) => {
            for line in lines {
                out.extend_from_slice(INDENT.as_bytes());
                out.extend_from_slice(&line);
                out.push(b'\n');
            }
        }
        None => {
            for line in source.lines() {
                out.extend_from_slice(INDENT.as_bytes());
                out.extend_from_slice(sanitize_line(line, opts.tab_width).as_bytes());
                out.push(b'\n');
            }
        }
    }
    let outputs: &[Value] = cell["outputs"].as_array().map_or(&[], Vec::as_slice);
    if outputs.is_empty() {
        return;
    }
    let number = outputs
        .iter()
        .find_map(|o| o["execution_count"].as_u64())
        .or_else(|| cell["execution_count"].as_u64())
        .map_or_else(|| " ".to_string(), |n| n.to_string());
    label(out, &format!("Out [{number}]:"), opts);
    for output in outputs {
        render_output(out, output, opts, hooks);
    }
}

fn render_output(out: &mut Vec<u8>, output: &Value, opts: &Options, hooks: &mut Hooks) {
    match output["output_type"].as_str() {
        Some("stream") => text_block(out, &text_of(&output["text"]), opts, hooks),
        Some("error") => {
            let name = text_of(&output["ename"]);
            let value = text_of(&output["evalue"]);
            let traceback: Vec<String> = output["traceback"]
                .as_array()
                .map(|items| {
                    items
                        .iter()
                        .filter_map(Value::as_str)
                        .map(String::from)
                        .collect()
                })
                .unwrap_or_default();
            if traceback.is_empty() {
                out.extend_from_slice(INDENT.as_bytes());
                if opts.colors {
                    out.extend_from_slice(BOLD_RED);
                }
                out.extend_from_slice(&(hooks.clean)(&format!("{name}: {value}")));
                if opts.colors {
                    out.extend_from_slice(RESET);
                }
                out.push(b'\n');
            } else {
                text_block(out, &traceback.join("\n"), opts, hooks);
            }
        }
        Some("execute_result" | "display_data") => {
            bundle_output(out, &output["data"], &output["metadata"], opts, hooks);
        }
        _ => {}
    }
}

fn bundle_output(out: &mut Vec<u8>, data: &Value, meta: &Value, opts: &Options, hooks: &mut Hooks) {
    let Some(data) = data.as_object() else {
        return;
    };
    for mime in ["image/png", "image/jpeg", "image/svg+xml"] {
        let Some(payload) = data.get(mime) else {
            continue;
        };
        let svg = mime == "image/svg+xml";
        let bytes = if svg {
            let text = text_of(payload);
            (text.len() <= MAX_IMAGE_BYTES).then(|| text.into_bytes())
        } else {
            decode_base64(&text_of(payload), MAX_IMAGE_BYTES)
        };
        let Some(bytes) = bytes else {
            placeholder(out, &format!("[{mime}]"), opts);
            return;
        };
        let size = image_size(mime, &bytes, &meta[mime]);
        match (hooks.image)(bytes, svg, max_cols(opts)) {
            Some(drawn) => {
                let _ = drawn.write(out);
            }
            None => match size {
                Some((w, h)) => placeholder(out, &format!("[{mime} {w}x{h}]"), opts),
                None => placeholder(out, &format!("[{mime}]"), opts),
            },
        }
        return;
    }
    if let Some(markdown) = data.get("text/markdown") {
        let text = text_of(markdown);
        let mut none = |_: &str, _: u16| None;
        out.extend_from_slice(&(hooks.markdown)(&text, &mut none));
    } else if let Some(plain) = data.get("text/plain") {
        text_block(out, &text_of(plain), opts, hooks);
    } else if data.contains_key("text/html") {
        placeholder(out, "[html output]", opts);
    } else if let Some(mime) = data.keys().next() {
        let name = sanitize_line(mime, 1);
        placeholder(out, &format!("[{name}]"), opts);
    }
}

fn placeholder(out: &mut Vec<u8>, text: &str, opts: &Options) {
    out.extend_from_slice(INDENT.as_bytes());
    dim_line(out, text, opts);
}

/// Pixel size from the metadata (`width`/`height`), or a PNG header.
fn image_size(mime: &str, bytes: &[u8], meta: &Value) -> Option<(u64, u64)> {
    if let (Some(w), Some(h)) = (meta["width"].as_u64(), meta["height"].as_u64()) {
        return Some((w, h));
    }
    if mime == "image/png" && bytes.len() >= 24 && bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        let w = u32::from_be_bytes(bytes[16..20].try_into().ok()?);
        let h = u32::from_be_bytes(bytes[20..24].try_into().ok()?);
        return Some((w.into(), h.into()));
    }
    None
}

/// Output text, indented, cut at [`MAX_OUTPUT_LINES`] lines.
fn text_block(out: &mut Vec<u8>, text: &str, opts: &Options, hooks: &mut Hooks) {
    let mut total = 0usize;
    for line in text.lines() {
        total += 1;
        if total > MAX_OUTPUT_LINES {
            continue;
        }
        // A carriage return rewinds the line: only the last part is visible.
        let visible = line
            .rsplit('\r')
            .find(|part| !part.is_empty())
            .unwrap_or("");
        let cleaned = (hooks.clean)(visible);
        out.extend_from_slice(INDENT.as_bytes());
        out.extend_from_slice(&cleaned);
        if opts.colors && cleaned.contains(&0x1b) {
            out.extend_from_slice(RESET);
        }
        out.push(b'\n');
    }
    if total > MAX_OUTPUT_LINES {
        let more = total - MAX_OUTPUT_LINES;
        let ellipsis = if opts.utf8 { "…" } else { "..." };
        placeholder(out, &format!("{ellipsis} {more} more lines"), opts);
    }
}

/// Decodes standard base64, ignoring whitespace. `None` for bad input or output over
/// `cap` bytes.
pub fn decode_base64(text: &str, cap: usize) -> Option<Vec<u8>> {
    if text.len() / 4 * 3 > cap + 3 {
        return None;
    }
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    let mut acc = 0u32;
    let mut bits = 0;
    let mut padding = false;
    for byte in text.bytes() {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' => {
                padding = true;
                continue;
            }
            b' ' | b'\n' | b'\r' | b'\t' => continue,
            _ => return None,
        };
        if padding {
            return None;
        }
        acc = (acc << 6) | u32::from(value);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    (out.len() <= cap).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_base64() {
        assert_eq!(decode_base64("aGVs\nbG8=", 100).unwrap(), b"hello");
        assert!(decode_base64("aGVsbG8=", 2).is_none());
        assert!(decode_base64("a$b=", 100).is_none());
    }

    #[test]
    fn rejects_other_formats() {
        assert!(parse(b"{}").is_err());
        assert!(parse(br#"{"nbformat":3,"cells":[]}"#).is_err());
        assert!(parse(br#"{"nbformat":4,"cells":[]}"#).is_ok());
    }

    #[test]
    fn detects_extension() {
        assert!(is_notebook(Some("a.ipynb"), None));
        assert!(!is_notebook(Some("a.json"), None));
        assert!(!is_notebook(Some("a.ipynb"), Some("python")));
    }
}
