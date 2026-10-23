//! Sizing math: how many cells and pixels an image gets.

/// Cell size assumed when the terminal does not report it (a 1:2 cell).
pub const DEFAULT_CELL: (u16, u16) = (8, 16);

/// The terminal size in cells: the window size of stdout, else `$COLUMNS`/`$LINES`,
/// else 80x24.
pub fn terminal_cells() -> (u16, u16) {
    if let Ok(size) = rustix::termios::tcgetwinsize(std::io::stdout())
        && size.ws_col > 0
        && size.ws_row > 0
    {
        return (size.ws_col, size.ws_row);
    }
    let env = |name: &str| {
        std::env::var(name)
            .ok()
            .and_then(|v| v.trim().parse::<u16>().ok())
            .filter(|&n| n > 0)
    };
    (env("COLUMNS").unwrap_or(80), env("LINES").unwrap_or(24))
}

/// Default maximum image height: the terminal height minus 4 rows, at least 10.
pub fn default_max_rows(term_rows: u16) -> u16 {
    term_rows.saturating_sub(4).max(10)
}

/// Placement of a pixel image (kitty, sixel): its pixel size and the cells it covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PixelFit {
    pub width: u32,
    pub height: u32,
    pub cols: u16,
    pub rows: u16,
}

/// Fits `width`x`height` pixels into `max_cols`x`max_rows` cells of `cell` pixels,
/// keeping the aspect ratio. Small images keep their native size unless `upscale`.
pub fn fit_pixels(
    width: u32,
    height: u32,
    max_cols: u16,
    max_rows: u16,
    cell: (u16, u16),
    upscale: bool,
) -> PixelFit {
    let (cw, ch) = (f64::from(cell.0.max(1)), f64::from(cell.1.max(1)));
    let (w, h) = (f64::from(width.max(1)), f64::from(height.max(1)));
    let box_w = f64::from(max_cols.max(1)) * cw;
    let box_h = f64::from(max_rows.max(1)) * ch;
    let mut scale = (box_w / w).min(box_h / h);
    if !upscale {
        scale = scale.min(1.0);
    }
    let px_w = (w * scale).round().clamp(1.0, box_w);
    let px_h = (h * scale).round().clamp(1.0, box_h);
    PixelFit {
        width: px_w as u32,
        height: px_h as u32,
        cols: ((px_w / cw).ceil() as u16).clamp(1, max_cols.max(1)),
        rows: ((px_h / ch).ceil() as u16).clamp(1, max_rows.max(1)),
    }
}

/// Fits an image into text cells, where every cell shows `sub` (x, y) sub-pixels.
/// Returns (cols, rows). Text output may upscale up to the limits.
pub fn fit_cells(
    width: u32,
    height: u32,
    max_cols: u16,
    max_rows: u16,
    cell: (u16, u16),
) -> (u16, u16) {
    let (cw, ch) = (f64::from(cell.0.max(1)), f64::from(cell.1.max(1)));
    // Rows needed per column so the picture keeps its shape on screen.
    let ratio = (f64::from(height.max(1)) / f64::from(width.max(1))) * (cw / ch);
    let mut cols = f64::from(max_cols.max(1));
    let mut rows = cols * ratio;
    if rows > f64::from(max_rows.max(1)) {
        rows = f64::from(max_rows.max(1));
        cols = rows / ratio;
    }
    let cols = (cols.round() as u16).clamp(1, max_cols.max(1));
    let rows = (rows.round() as u16).clamp(1, max_rows.max(1));
    (cols, rows)
}

/// Thumbnails (cover art, poster frames) are at most this many rows high.
const THUMB_MAX_ROWS: u16 = 20;
/// Below this many rows a thumbnail is left out.
const THUMB_MIN_ROWS: u16 = 4;

/// The cell box for a thumbnail drawn above or beside an info table on a terminal of
/// `cols`x`rows`: at most `max_cols` wide and 20 rows or a third of the terminal
/// high, whichever is smaller. `None` when the terminal is too small to bother.
pub fn thumbnail_box(cols: u16, rows: u16, max_cols: u16) -> Option<(u16, u16)> {
    let max_rows = THUMB_MAX_ROWS.min(rows / 3);
    (max_rows >= THUMB_MIN_ROWS).then_some((max_cols.min(cols).max(1), max_rows))
}

/// Columns and rows below which text output may still enlarge a small image to show
/// each of its pixels, so icons stay recognisable.
const TEXT_FLOOR: (u16, u16) = (32, 16);

/// The largest cell box text output may use for an image of `width`x`height` pixels,
/// so small images are not blown up to the whole screen. The box is the larger of
/// twice the image's size in cells (what kitty or sixel would cover, doubled) and one
/// sub-pixel per image pixel, the latter limited to [`TEXT_FLOOR`].
pub fn text_cap(width: u32, height: u32, cell: (u16, u16), sub: (u32, u32)) -> (u16, u16) {
    let cap = |px: u32, cell: u16, sub: u32, floor: u16| -> u16 {
        let doubled = (u64::from(px) * 2).div_ceil(u64::from(cell.max(1)));
        let native = u64::from(px)
            .div_ceil(u64::from(sub.max(1)))
            .min(u64::from(floor));
        doubled.max(native).clamp(1, u64::from(u16::MAX)) as u16
    };
    (
        cap(width, cell.0, sub.0, TEXT_FLOOR.0),
        cap(height, cell.1, sub.1, TEXT_FLOOR.1),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn large_images_shrink_to_the_width() {
        let fit = fit_pixels(4000, 3000, 100, 50, (10, 20), false);
        assert_eq!((fit.width, fit.height), (1000, 750));
        assert_eq!((fit.cols, fit.rows), (100, 38));
    }

    #[test]
    fn tall_images_shrink_to_the_height() {
        let fit = fit_pixels(1000, 4000, 100, 20, (10, 20), false);
        assert_eq!((fit.width, fit.height), (100, 400));
        assert_eq!((fit.cols, fit.rows), (10, 20));
    }

    #[test]
    fn small_images_are_not_upscaled() {
        let fit = fit_pixels(16, 16, 100, 50, (10, 20), false);
        assert_eq!((fit.width, fit.height), (16, 16));
        assert_eq!((fit.cols, fit.rows), (2, 1));
        let fit = fit_pixels(16, 16, 100, 50, (10, 20), true);
        assert_eq!((fit.width, fit.height), (1000, 1000));
    }

    #[test]
    fn cells_keep_aspect_with_tall_cells() {
        // A square image on 1:2 cells needs half as many rows as columns.
        assert_eq!(fit_cells(100, 100, 80, 100, (8, 16)), (80, 40));
        // Height-limited: 20 rows, so 40 columns.
        assert_eq!(fit_cells(100, 100, 80, 20, (8, 16)), (40, 20));
        // Wide image.
        assert_eq!(fit_cells(400, 100, 80, 40, (8, 16)), (80, 10));
        // Tiny images may grow (text output).
        assert_eq!(fit_cells(4, 4, 80, 10, (8, 16)), (20, 10));
    }

    #[test]
    fn text_output_caps_small_images() {
        // A 16x16 icon on half blocks: one sub-pixel per pixel, 16x8 cells.
        assert_eq!(text_cap(16, 16, (8, 16), (1, 2)), (16, 8));
        // Sextants need fewer cells for the same pixels.
        assert_eq!(text_cap(16, 16, (8, 16), (2, 3)), (8, 6));
        // 200 px: twice its 25x13 cells-equivalent.
        assert_eq!(text_cap(200, 200, (8, 16), (1, 2)), (50, 25));
        // A 1x1 pixel stays one cell.
        assert_eq!(text_cap(1, 1, (8, 16), (1, 2)), (1, 1));
        // Through fit_cells, a 16x16 icon stays 16x8 on a big terminal.
        let (c, r) = text_cap(16, 16, (8, 16), (1, 2));
        assert_eq!(fit_cells(16, 16, 200.min(c), 60.min(r), (8, 16)), (16, 8));
    }

    #[test]
    fn thumbnails_are_modest() {
        // Tall terminal: the fixed maximum.
        assert_eq!(thumbnail_box(200, 80, 40), Some((40, 20)));
        // 45 rows: a third.
        assert_eq!(thumbnail_box(120, 45, 40), Some((40, 15)));
        // Narrow terminal: its width.
        assert_eq!(thumbnail_box(30, 60, 40), Some((30, 20)));
        // Tiny terminal: none.
        assert_eq!(thumbnail_box(80, 11, 40), None);
    }

    #[test]
    fn default_height_leaves_room() {
        assert_eq!(default_max_rows(50), 46);
        assert_eq!(default_max_rows(8), 10);
    }
}
