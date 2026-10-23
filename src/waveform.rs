//! Peak waveforms drawn with braille dots (2x4 per cell) or ASCII.

use std::io::{self, Write};

/// Rows of text the waveform occupies. Must be even: the wave is mirrored around the
/// line between the two middle rows.
pub const ROWS: usize = 4;

const CYAN: &str = "\x1b[36m";
const RESET: &str = "\x1b[0m";

/// Braille dot bits by (column, row) inside a cell; see Unicode block U+2800.
const BRAILLE_BITS: [[u8; 4]; 2] = [[0x01, 0x02, 0x04, 0x40], [0x08, 0x10, 0x20, 0x80]];

/// Reduces `peaks` (any length, values 0..=1) to `columns` values by taking the
/// maximum of each slice. With fewer peaks than columns, peaks are repeated.
pub fn resample(peaks: &[f32], columns: usize) -> Vec<f32> {
    if peaks.is_empty() || columns == 0 {
        return vec![0.0; columns];
    }
    let len = peaks.len();
    (0..columns)
        .map(|c| {
            let start = c * len / columns;
            let end = ((c + 1) * len / columns).max(start + 1).min(len);
            peaks[start..end].iter().copied().fold(0.0, f32::max)
        })
        .collect()
}

/// Scales peaks so the loudest one reaches 1.0. Silence stays silence.
pub fn normalize(peaks: &mut [f32]) {
    let max = peaks.iter().copied().fold(0.0, f32::max);
    if max > 0.0 {
        for p in peaks.iter_mut() {
            *p = (*p / max).clamp(0.0, 1.0);
        }
    }
}

/// How many vertical units (dots or half rows) one side of the centre line gets for
/// amplitude `a` out of `max` units. Never zero, so silence still shows the centre line.
fn units(a: f32, max: usize) -> usize {
    ((a.clamp(0.0, 1.0) * max as f32).round() as usize).clamp(1, max)
}

/// Braille waveform, `width` cells wide and `ROWS` rows tall. `peaks` are resampled to
/// two dot columns per cell.
pub fn braille_lines(peaks: &[f32], width: usize) -> Vec<String> {
    let dot_rows = ROWS * 4;
    let half = dot_rows / 2;
    let columns = resample(peaks, width * 2);
    let mut cells = vec![vec![0u8; width]; ROWS];
    for (x, &a) in columns.iter().enumerate() {
        let n = units(a, half);
        for dot_row in half - n..half + n {
            cells[dot_row / 4][x / 2] |= BRAILLE_BITS[x % 2][dot_row % 4];
        }
    }
    cells
        .into_iter()
        .map(|row| {
            row.into_iter()
                .map(|bits| char::from_u32(0x2800 + u32::from(bits)).unwrap_or(' '))
                .collect()
        })
        .collect()
}

/// ASCII waveform for terminals without UTF-8: one column per cell, two levels per row
/// (`#` full, `:` half). Silence shows as `.` just above the centre line.
pub fn ascii_lines(peaks: &[f32], width: usize) -> Vec<String> {
    let half_rows = ROWS / 2;
    let columns = resample(peaks, width);
    let mut grid = vec![vec![' '; width]; ROWS];
    for (x, &a) in columns.iter().enumerate() {
        // Half-row units on each side of the centre.
        let n = ((a.clamp(0.0, 1.0) * (half_rows * 2) as f32).round()) as usize;
        if n == 0 {
            grid[half_rows - 1][x] = '.';
            continue;
        }
        for step in 0..half_rows {
            // How much of the row `step` rows away from the centre is filled: 0, 1 or 2 halves.
            let filled = n.saturating_sub(step * 2).min(2);
            let ch = match filled {
                2 => '#',
                1 => ':',
                _ => continue,
            };
            grid[half_rows - 1 - step][x] = ch;
            grid[half_rows + step][x] = ch;
        }
    }
    grid.into_iter()
        .map(|row| row.into_iter().collect::<String>().trim_end().to_string())
        .collect()
}

pub fn write<W: Write>(
    out: &mut W,
    peaks: &[f32],
    width: usize,
    utf8: bool,
    colors: bool,
) -> io::Result<()> {
    let lines = if utf8 {
        braille_lines(peaks, width)
    } else {
        ascii_lines(peaks, width)
    };
    let (on, off) = if colors { (CYAN, RESET) } else { ("", "") };
    for line in lines {
        writeln!(out, "{on}{line}{off}")?;
    }
    Ok(())
}

/// Terminal width in cells: the stdout window size, else `$COLUMNS`, else 80.
pub fn terminal_width() -> usize {
    if let Ok(size) = rustix::termios::tcgetwinsize(io::stdout())
        && size.ws_col > 0
    {
        return usize::from(size.ws_col);
    }
    std::env::var("COLUMNS")
        .ok()
        .and_then(|c| c.trim().parse::<usize>().ok())
        .filter(|&c| c > 0)
        .unwrap_or(80)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resample_takes_slice_maxima() {
        assert_eq!(resample(&[0.1, 0.9, 0.2, 0.3], 2), vec![0.9, 0.3]);
        assert_eq!(resample(&[0.5, 1.0], 4), vec![0.5, 0.5, 1.0, 1.0]);
        assert_eq!(resample(&[], 3), vec![0.0; 3]);
    }

    #[test]
    fn normalizes_to_loudest_peak() {
        let mut peaks = vec![0.25, 0.5, 0.0];
        normalize(&mut peaks);
        assert_eq!(peaks, vec![0.5, 1.0, 0.0]);
        let mut silent = vec![0.0, 0.0];
        normalize(&mut silent);
        assert_eq!(silent, vec![0.0, 0.0]);
    }

    #[test]
    fn braille_full_and_silent_cells() {
        // Full amplitude in both dot columns fills every dot of every row.
        let lines = braille_lines(&[1.0, 1.0], 1);
        assert_eq!(lines, vec!["⣿", "⣿", "⣿", "⣿"]);
        // Silence: one dot each side of the centre, in both columns.
        let lines = braille_lines(&[0.0, 0.0], 1);
        assert_eq!(
            lines,
            vec![
                "\u{2800}",
                char::from_u32(0x2800 + 0x40 + 0x80)
                    .unwrap()
                    .to_string()
                    .as_str(),
                char::from_u32(0x2800 + 0x01 + 0x08)
                    .unwrap()
                    .to_string()
                    .as_str(),
                "\u{2800}",
            ]
        );
    }

    #[test]
    fn braille_composes_left_and_right_columns() {
        // Left column full, right column silent: the middle rows show the left column
        // fully plus the right column's single centre dot.
        let lines = braille_lines(&[1.0, 0.0], 1);
        let left = 0x01 | 0x02 | 0x04 | 0x40;
        let mid_top = char::from_u32(0x2800 + left + 0x80).unwrap().to_string();
        let mid_bottom = char::from_u32(0x2800 + left + 0x08).unwrap().to_string();
        let outer = char::from_u32(0x2800 + left).unwrap().to_string();
        assert_eq!(lines, vec![outer.clone(), mid_top, mid_bottom, outer]);
    }

    #[test]
    fn braille_is_mirrored() {
        let lines = braille_lines(&[0.3, 0.6, 0.9, 0.1], 2);
        assert_eq!(lines.len(), ROWS);
        let flip = |c: char| {
            // Mirror a braille cell vertically: dot rows 0<->3, 1<->2.
            let b = c as u32 - 0x2800;
            let bit = |n: u32| (b >> n) & 1;
            let m = bit(6)
                | bit(2) << 1
                | bit(1) << 2
                | bit(0) << 6
                | bit(7) << 3
                | bit(5) << 4
                | bit(4) << 5
                | bit(3) << 7;
            char::from_u32(0x2800 + m).unwrap()
        };
        for (top, bottom) in [(0, 3), (1, 2)] {
            let mirrored: String = lines[top].chars().map(flip).collect();
            assert_eq!(mirrored, lines[bottom]);
        }
    }

    #[test]
    fn ascii_fallback() {
        let lines = ascii_lines(&[1.0, 0.5, 0.25, 0.0], 4);
        assert_eq!(lines, vec!["#", "##:.", "##:", "#"]);
        let lines = ascii_lines(&[0.375], 1);
        // 0.375 * 4 = 1.5, rounds to 2 half rows: the rows next to the centre are full.
        assert_eq!(lines, vec!["", "#", "#", ""]);
        assert!(ascii_lines(&[0.9; 10], 10).iter().all(|l| l.is_ascii()));
    }
}
