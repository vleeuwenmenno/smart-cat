//! Spreadsheets (xlsx, xlsm, xls, ods): one table per sheet, read with calamine.

use std::io::{Cursor, Read, Seek};

use calamine::{Data, DataType, ExcelDateTime, Ods, Reader, Xls, Xlsx, open_workbook_from_rs};

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
            // Streamed cell by cell: `worksheet_range` would first build the dense
            // range, and two cells at A1 and XFD1048576 make that 2^34 cells.
            tables(&mut book, limits, |book, name, grid| {
                let mut cells = book
                    .worksheet_cells_reader(name)
                    .map_err(|e| format!("sheet {name}: {e}"))?;
                while let Some(cell) = cells
                    .next_cell()
                    .map_err(|e| format!("sheet {name}: {e}"))?
                {
                    if cell.get_value().is_empty() {
                        continue;
                    }
                    let (row, col) = cell.get_position();
                    grid.add(row, col, || Data::from(cell.get_value().clone()));
                }
                Ok(())
            })
        }
        // Ods and xls hold their sheets in memory once opened; the helper's memory
        // limit is the backstop there.
        Format::Ods => {
            let mut book: Ods<_> = open_workbook_from_rs(cursor).map_err(|e| err(&e))?;
            tables(&mut book, limits, from_range)
        }
        _ => {
            let mut book: Xls<_> = open_workbook_from_rs(cursor).map_err(|e| err(&e))?;
            tables(&mut book, limits, from_range)
        }
    }
}

/// Fills `grid` from the sheet's range (formats that build it anyway).
fn from_range<RS, R>(book: &mut R, name: &str, grid: &mut Grid) -> Result<(), String>
where
    RS: Read + Seek,
    R: Reader<RS>,
    R::Error: std::fmt::Display,
{
    let range = book
        .worksheet_range(name)
        .map_err(|err| format!("sheet {name}: {err}"))?;
    let (row0, col0) = range.start().unwrap_or((0, 0));
    for (row, col, data) in range.used_cells() {
        grid.add(row0 + row as u32, col0 + col as u32, || data.clone());
    }
    Ok(())
}

/// Hard cap on table rows even when `max_rows` is 0 (all): the markdown stops growing
/// long before, and empty rows between sparse cells still cost memory.
const HARD_MAX_ROWS: usize = 50_000;

/// The cells of one sheet that can be shown: at most `rows` x [`MAX_COLUMNS`] from the
/// top-left used cell, plus the extent of the whole sheet for the "more" notes.
/// Memory stays proportional to the shown part, however sparse the sheet is.
struct Grid {
    rows: usize,
    /// (row, column, value), absolute positions.
    cells: Vec<(u32, u32, Data)>,
    min: Option<(u32, u32)>,
    max: (u32, u32),
}

impl Grid {
    fn new(rows: usize) -> Self {
        Grid {
            rows,
            cells: Vec::new(),
            min: None,
            max: (0, 0),
        }
    }

    /// Adds a used (non-empty) cell; `value` is only made when the cell is shown.
    fn add(&mut self, row: u32, col: u32, value: impl FnOnce() -> Data) {
        let (min_row, min_col) = match self.min {
            None => (row, col),
            Some((r, c)) => (r.min(row), c.min(col)),
        };
        if self.min != Some((min_row, min_col)) {
            self.min = Some((min_row, min_col));
            let (rows, cols) = (self.rows, MAX_COLUMNS);
            self.cells.retain(|&(r, c, _)| {
                ((r - min_row) as usize) < rows && ((c - min_col) as usize) < cols
            });
        }
        self.max = (self.max.0.max(row), self.max.1.max(col));
        if ((row - min_row) as usize) < self.rows && ((col - min_col) as usize) < MAX_COLUMNS {
            self.cells.push((row, col, value()));
        }
    }

    /// (height, width) of the used area.
    fn size(&self) -> (usize, usize) {
        match self.min {
            None => (0, 0),
            Some((r, c)) => ((self.max.0 - r) as usize + 1, (self.max.1 - c) as usize + 1),
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

fn tables<RS, R>(
    book: &mut R,
    limits: &Limits,
    read: impl Fn(&mut R, &str, &mut Grid) -> Result<(), String>,
) -> Result<Converted, String>
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
        // The header row plus the data rows.
        let rows = if limits.max_rows == 0 {
            HARD_MAX_ROWS
        } else {
            limits.max_rows.saturating_add(1).min(HARD_MAX_ROWS)
        };
        let mut grid = Grid::new(rows);
        read(book, name, &mut grid)?;
        write_sheet(&mut md, &grid);
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

fn write_sheet(md: &mut Md, grid: &Grid) {
    let (height, width) = grid.size();
    let Some((row0, col0)) = grid.min.filter(|_| !grid.cells.is_empty()) else {
        md.paragraph("*(empty sheet)*");
        return;
    };
    let shown_columns = width.min(MAX_COLUMNS);
    let shown_rows = height.min(grid.rows);
    let mut rows = vec![vec![String::new(); shown_columns]; shown_rows];
    for (r, c, value) in &grid.cells {
        rows[(r - row0) as usize][(c - col0) as usize] = cell(value);
    }
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
