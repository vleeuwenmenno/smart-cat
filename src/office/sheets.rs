//! Spreadsheets (xlsx, xlsm, xls, ods): one table per sheet, read with calamine.

use std::io::{Cursor, Read, Seek};

use calamine::{Data, ExcelDateTime, Ods, Range, Reader, Xls, Xlsx, open_workbook_from_rs};

use super::md::{Md, cell_text};
use super::{Converted, Format, Limits, MAX_COLUMNS, MAX_SHEET_BYTES, MAX_SHEETS};

pub fn convert(format: Format, data: &[u8], limits: &Limits) -> Result<Converted, String> {
    if format != Format::Xls {
        check_declared_sizes(data)?;
    }
    let cursor = Cursor::new(data);
    let err = |e: &dyn std::fmt::Display| e.to_string();
    match format {
        Format::Xlsx => {
            let mut book: Xlsx<_> = open_workbook_from_rs(cursor).map_err(|e| err(&e))?;
            tables(&mut book, limits)
        }
        Format::Ods => {
            let mut book: Ods<_> = open_workbook_from_rs(cursor).map_err(|e| err(&e))?;
            tables(&mut book, limits)
        }
        _ => {
            let mut book: Xls<_> = open_workbook_from_rs(cursor).map_err(|e| err(&e))?;
            tables(&mut book, limits)
        }
    }
}

/// Refuses archives whose parts claim to unpack to more than the cap.
fn check_declared_sizes(data: &[u8]) -> Result<(), String> {
    let mut zip =
        zip::ZipArchive::new(Cursor::new(data)).map_err(|err| format!("not a zip file: {err}"))?;
    let mut total = 0u64;
    for i in 0..zip.len() {
        let file = zip.by_index_raw(i).map_err(|err| err.to_string())?;
        total = total.saturating_add(file.size());
    }
    if total > MAX_SHEET_BYTES {
        return Err(format!(
            "unpacks to more than {}",
            crate::util::human_size(MAX_SHEET_BYTES)
        ));
    }
    Ok(())
}

fn tables<RS, R>(book: &mut R, limits: &Limits) -> Result<Converted, String>
where
    RS: Read + Seek,
    R: Reader<RS>,
    R::Error: std::fmt::Display,
{
    let visible: Vec<String> = book
        .sheets_metadata()
        .iter()
        .filter(|s| s.visible == calamine::SheetVisible::Visible)
        .map(|s| s.name.clone())
        .collect();
    let mut md = Md::new();
    for name in visible.iter().take(MAX_SHEETS) {
        if md.is_full() {
            break;
        }
        md.heading(2, &cell_text(name));
        let range = book
            .worksheet_range(name)
            .map_err(|err| format!("sheet {name}: {err}"))?;
        write_sheet(&mut md, &range, limits);
    }
    if visible.len() > MAX_SHEETS {
        md.paragraph(&format!("*… {} more sheets*", visible.len() - MAX_SHEETS));
    }
    let counts = match visible.len() {
        1 => "1 sheet".to_string(),
        n => format!("{n} sheets"),
    };
    Ok(Converted {
        markdown: md.finish(),
        counts,
    })
}

fn write_sheet(md: &mut Md, range: &Range<Data>, limits: &Limits) {
    let (height, width) = range.get_size();
    if height == 0 || width == 0 {
        md.paragraph("*(empty sheet)*");
        return;
    }
    let shown_columns = width.min(MAX_COLUMNS);
    // The header row plus the data rows.
    let shown_rows = if limits.max_rows == 0 {
        height
    } else {
        height.min(limits.max_rows.saturating_add(1))
    };
    let rows: Vec<Vec<String>> = range
        .rows()
        .take(shown_rows)
        .map(|row| row.iter().take(shown_columns).map(cell).collect())
        .collect();
    md.table(&rows);
    if shown_rows < height {
        md.paragraph(&format!("*… {} more rows*", height - shown_rows));
    }
    if shown_columns < width {
        md.paragraph(&format!("*… {} more columns*", width - shown_columns));
    }
}

fn cell(data: &Data) -> String {
    match data {
        Data::Empty => String::new(),
        Data::Int(n) => n.to_string(),
        Data::Float(f) => float(*f),
        Data::String(s) | Data::DateTimeIso(s) | Data::DurationIso(s) => cell_text(s),
        Data::Bool(b) => if *b { "TRUE" } else { "FALSE" }.to_string(),
        Data::DateTime(dt) => date_time(dt),
        Data::Error(e) => cell_text(&e.to_string()),
    }
}

fn float(f: f64) -> String {
    if f.is_finite() && f.fract() == 0.0 && f.abs() < 1e15 {
        format!("{}", f as i64)
    } else {
        f.to_string()
    }
}

fn date_time(dt: &ExcelDateTime) -> String {
    if dt.is_duration() {
        let total = (dt.as_f64() * 86_400.0).round() as i64;
        return format!("{}:{:02}:{:02}", total / 3600, total / 60 % 60, total % 60);
    }
    if !dt.is_datetime() {
        return float(dt.as_f64());
    }
    let (y, mo, d, h, mi, s, _) = dt.to_ymd_hms_milli();
    let time = format!("{h:02}:{mi:02}:{s:02}");
    if dt.as_f64() < 1.0 {
        time
    } else if (h, mi, s) == (0, 0, 0) {
        format!("{y:04}-{mo:02}-{d:02}")
    } else {
        format!("{y:04}-{mo:02}-{d:02} {time}")
    }
}
