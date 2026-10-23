//! Office documents (docx, odt, pptx, odp, xlsx, xlsm, xls, ods) shown as markdown.
//!
//! The converters read the zip container with the `zip` crate, the XML with
//! `quick-xml` and spreadsheets with `calamine`. They produce a markdown string that
//! the markdown renderer draws. Everything in a document is untrusted: decompressed
//! parts, row counts, slide counts and the markdown size are capped, and text is
//! stripped of control characters before it reaches the markdown.
//!
//! calamine (spreadsheets) can panic on hostile files, which aborts a release build.
//! Spreadsheets are therefore converted in a sandboxed child process, scat itself
//! started with [`crate::sandbox::SHEET_FLAG`]; see [`crate::sandbox`].

mod docx;
mod md;
mod odf;
mod pptx;
mod sheets;
mod xml;

use std::io::{self, Cursor, Read, Seek, Write};

use crate::input::{Input, InputKind};
use crate::util::human_size;

/// Documents larger than this are refused (the archive is read into memory).
const MAX_FILE_BYTES: u64 = 256 * 1024 * 1024;
/// One decompressed XML part is read up to this size; more is an error.
const MAX_PART_BYTES: u64 = 64 * 1024 * 1024;
/// All decompressed parts of one spreadsheet together, checked on the declared sizes.
const MAX_SHEET_BYTES: u64 = 512 * 1024 * 1024;
/// Wall clock limit for converting one spreadsheet in the helper.
const SHEET_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);
/// Address space limit for the spreadsheet helper: the file itself, the decompressed
/// parts calamine keeps, and the markdown.
const SHEET_MEMORY: u64 = 2 << 30;
/// The markdown of one document stops growing here.
const MAX_MARKDOWN_BYTES: usize = 8 * 1024 * 1024;
/// At most this many slides are converted.
pub const MAX_SLIDES: usize = 200;
/// Spreadsheet columns shown per sheet.
pub const MAX_COLUMNS: usize = 64;
/// Spreadsheets with more sheets than this show the first ones.
pub const MAX_SHEETS: usize = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Docx,
    Odt,
    Pptx,
    Odp,
    Xlsx,
    Xls,
    Ods,
}

impl Format {
    fn label(self) -> &'static str {
        match self {
            Format::Docx => "Word document (docx)",
            Format::Odt => "OpenDocument text (odt)",
            Format::Pptx => "PowerPoint presentation (pptx)",
            Format::Odp => "OpenDocument presentation (odp)",
            Format::Xlsx => "Excel workbook (xlsx)",
            Format::Xls => "Excel workbook (xls)",
            Format::Ods => "OpenDocument spreadsheet (ods)",
        }
    }

    /// The helper's name for a spreadsheet format.
    fn sheet_arg(self) -> Option<&'static str> {
        match self {
            Format::Xlsx => Some("xlsx"),
            Format::Xls => Some("xls"),
            Format::Ods => Some("ods"),
            _ => None,
        }
    }

    fn from_extension(ext: &str) -> Option<Self> {
        Some(match ext {
            "docx" | "docm" => Format::Docx,
            "odt" => Format::Odt,
            "pptx" | "pptm" => Format::Pptx,
            "odp" => Format::Odp,
            "xlsx" | "xlsm" => Format::Xlsx,
            "xls" => Format::Xls,
            "ods" => Format::Ods,
            _ => return None,
        })
    }

    fn from_mime(mime: &str) -> Option<Self> {
        Some(match mime {
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document" => {
                Format::Docx
            }
            "application/vnd.openxmlformats-officedocument.presentationml.presentation" => {
                Format::Pptx
            }
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet" => Format::Xlsx,
            "application/vnd.oasis.opendocument.text" => Format::Odt,
            "application/vnd.oasis.opendocument.presentation" => Format::Odp,
            "application/vnd.oasis.opendocument.spreadsheet" => Format::Ods,
            "application/vnd.ms-excel" => Format::Xls,
            _ => return None,
        })
    }
}

const ZIP_MAGIC: &[u8] = b"PK\x03\x04";
const OLE_MAGIC: &[u8] = b"\xd0\xcf\x11\xe0\xa1\xb1\x1a\xe1";

/// The document format a file is, going by its extension and container magic, or by
/// the MIME type sniffed from the first bytes.
pub fn wants(mime: &str, file_name: Option<&str>, head: &[u8]) -> Option<Format> {
    let by_ext = file_name
        .and_then(|name| name.rsplit_once('.'))
        .and_then(|(_, ext)| Format::from_extension(&ext.to_ascii_lowercase()));
    let container_ok = |format: Format| match format {
        Format::Xls => head.starts_with(OLE_MAGIC),
        _ => head.starts_with(ZIP_MAGIC),
    };
    match by_ext {
        Some(format) if container_ok(format) => Some(format),
        // A wrong extension on a zip is the archive view's business.
        Some(_) => None,
        None => Format::from_mime(mime).filter(|&format| container_ok(format)),
    }
}

/// Everything a conversion needs from the configuration.
pub struct Limits {
    /// Data rows per sheet; 0 shows all.
    pub max_rows: usize,
}

pub struct Converted {
    pub markdown: String,
    /// "12 pages", "3 slides", "2 sheets"; empty when unknown.
    pub counts: String,
}

/// Header line text: `Word document (docx), 12 pages, 48.2 KiB`.
pub fn header_text(format: Format, converted: &Converted, size: Option<u64>) -> String {
    let mut text = format.label().to_string();
    if !converted.counts.is_empty() {
        text.push_str(", ");
        text.push_str(&converted.counts);
    }
    if let Some(size) = size {
        text.push_str(", ");
        text.push_str(&human_size(size));
    }
    text
}

/// Reads the document into memory. Regular files are checked against the size cap first.
pub fn read_input(input: &mut Input) -> io::Result<Vec<u8>> {
    if let InputKind::Regular { size } = input.kind
        && size > MAX_FILE_BYTES
    {
        return Err(io::Error::other(format!(
            "cannot read document: file is larger than {}",
            human_size(MAX_FILE_BYTES)
        )));
    }
    let mut data = Vec::new();
    (&mut input.reader)
        .take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut data)?;
    if data.len() as u64 > MAX_FILE_BYTES {
        return Err(io::Error::other(format!(
            "cannot read document: input is larger than {}",
            human_size(MAX_FILE_BYTES)
        )));
    }
    Ok(data)
}

/// Converts a document to markdown. Errors are the text after `cannot read document: `.
pub fn convert(format: Format, data: &[u8], limits: &Limits) -> Result<Converted, String> {
    match format {
        Format::Docx => docx::convert(open_zip(data)?),
        Format::Pptx => pptx::convert(open_zip(data)?),
        Format::Odt | Format::Odp => odf::convert(open_zip(data)?, format),
        Format::Xlsx | Format::Xls | Format::Ods => convert_sheet(format, data, limits),
    }
}

/// Converts a spreadsheet in the sandboxed helper. The document goes to its standard
/// input; it answers with the counts on the first line and the markdown after it.
fn convert_sheet(format: Format, data: &[u8], limits: &Limits) -> Result<Converted, String> {
    use crate::sandbox;
    let name = format.sheet_arg().ok_or("not a spreadsheet")?;
    let cmd = sandbox::helper(sandbox::SHEET_FLAG, [name, &limits.max_rows.to_string()]);
    let out = sandbox::run_with(
        cmd,
        sandbox::Limits {
            timeout: SHEET_TIMEOUT,
            memory: SHEET_MEMORY,
            stdin: Some(data),
        },
    )
    .map_err(|err| err.to_string())?;
    let text = String::from_utf8(out).map_err(|_| "the helper's output is not UTF-8")?;
    let (counts, markdown) = text
        .split_once('\n')
        .ok_or("the helper's output is incomplete")?;
    Ok(Converted {
        counts: counts.to_string(),
        markdown: markdown.to_string(),
    })
}

/// Runs the spreadsheet helper when scat was started as one (`scat --internal-sheet
/// FORMAT MAX_ROWS`, the document on standard input); `None` for a normal run.
pub fn internal_main() -> Option<i32> {
    let mut args = std::env::args_os().skip(1);
    if args.next()? != crate::sandbox::SHEET_FLAG {
        return None;
    }
    crate::sandbox::report_panics("spreadsheet parser");
    let args: Vec<String> = args.filter_map(|a| a.into_string().ok()).collect();
    let result = (|| {
        let format = match args.first().map(String::as_str) {
            Some("xlsx") => Format::Xlsx,
            Some("xls") => Format::Xls,
            Some("ods") => Format::Ods,
            _ => return Err("usage error".to_string()),
        };
        let max_rows = args
            .get(1)
            .and_then(|n| n.parse().ok())
            .ok_or("usage error")?;
        let mut data = Vec::new();
        io::stdin()
            .lock()
            .take(MAX_FILE_BYTES + 1)
            .read_to_end(&mut data)
            .map_err(|err| crate::util::io_message(&err))?;
        if data.len() as u64 > MAX_FILE_BYTES {
            return Err(format!(
                "input is larger than {}",
                human_size(MAX_FILE_BYTES)
            ));
        }
        sheets::convert(format, &data, &Limits { max_rows })
    })();
    match result {
        Ok(converted) => {
            let mut stdout = io::stdout().lock();
            let written = writeln!(stdout, "{}", converted.counts)
                .and_then(|()| stdout.write_all(converted.markdown.as_bytes()))
                .and_then(|()| stdout.flush());
            Some(if written.is_ok() { 0 } else { 1 })
        }
        Err(message) => {
            eprintln!("scat: {message}");
            Some(2)
        }
    }
}

type Zip<'a> = zip::ZipArchive<Cursor<&'a [u8]>>;

fn open_zip(data: &[u8]) -> Result<Zip<'_>, String> {
    zip::ZipArchive::new(Cursor::new(data)).map_err(|err| format!("not a zip file: {err}"))
}

/// Reads one part of the container, capped; `None` when it does not exist.
fn read_part<R: Read + Seek>(
    zip: &mut zip::ZipArchive<R>,
    name: &str,
) -> Result<Option<Vec<u8>>, String> {
    let file = match zip.by_name(name) {
        Ok(file) => file,
        Err(zip::result::ZipError::FileNotFound) => return Ok(None),
        Err(err) => return Err(format!("{name}: {err}")),
    };
    let mut data = Vec::new();
    file.take(MAX_PART_BYTES + 1)
        .read_to_end(&mut data)
        .map_err(|err| format!("{name}: {err}"))?;
    if data.len() as u64 > MAX_PART_BYTES {
        return Err(format!(
            "{name}: larger than {} when decompressed",
            human_size(MAX_PART_BYTES)
        ));
    }
    Ok(Some(data))
}

fn require_part<R: Read + Seek>(
    zip: &mut zip::ZipArchive<R>,
    name: &str,
) -> Result<Vec<u8>, String> {
    read_part(zip, name)?.ok_or_else(|| format!("missing {name}, not a valid document"))
}

/// Writes the dim header line (unless `plain`), then a blank line.
pub fn write_header<W: Write>(
    out: &mut W,
    text: &str,
    colors: bool,
    plain: bool,
) -> io::Result<()> {
    if plain {
        return Ok(());
    }
    let text = crate::render::sanitize_str(text);
    if colors {
        write!(out, "\x1b[2m{text}\x1b[22m\n\n")
    } else {
        write!(out, "{text}\n\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_by_extension_and_magic() {
        assert_eq!(wants("", Some("a.DOCX"), b"PK\x03\x04"), Some(Format::Docx));
        assert_eq!(wants("", Some("a.xls"), OLE_MAGIC), Some(Format::Xls));
        assert_eq!(wants("", Some("a.xls"), b"PK\x03\x04"), None);
        assert_eq!(wants("", Some("a.docx"), b"hello"), None);
        assert_eq!(wants("", Some("a.zip"), b"PK\x03\x04"), None);
        assert_eq!(
            wants(
                "application/vnd.oasis.opendocument.text",
                None,
                b"PK\x03\x04"
            ),
            Some(Format::Odt)
        );
    }
}
