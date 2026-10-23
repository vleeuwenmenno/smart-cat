//! The rule drawn before each file when several files are rendered.

use unicode_width::UnicodeWidthStr;

use crate::render::sanitize_str;

/// `── name ─────────` filling `width` columns, dim when colours are on, ending in a
/// newline. The name is sanitized and shortened (from the left) when it does not fit.
pub fn line(name: &str, width: usize, colors: bool, utf8: bool) -> String {
    let (rule, lead) = if utf8 {
        ('─', "── ")
    } else {
        ('-', "-- ")
    };
    let mut name = sanitize_str(name);
    // Leave room for the lead, a space after the name, and at least one rule character.
    let room = width.saturating_sub(lead.width() + 2).max(1);
    if name.width() > room {
        let ellipsis = if utf8 { "…" } else { "..." };
        let keep = room.saturating_sub(ellipsis.width());
        let mut tail: Vec<char> = Vec::new();
        let mut used = 0;
        for c in name.chars().rev() {
            let w = unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
            if used + w > keep {
                break;
            }
            used += w;
            tail.push(c);
        }
        tail.reverse();
        name = format!("{ellipsis}{}", tail.into_iter().collect::<String>());
    }
    let used = lead.width() + name.width() + 1;
    let fill: String = std::iter::repeat_n(rule, width.saturating_sub(used).max(1)).collect();
    let text = format!("{lead}{name} {fill}");
    if colors {
        format!("\x1b[2m{text}\x1b[22m\n")
    } else {
        format!("{text}\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fills_the_width() {
        assert_eq!(
            line("src/main.rs", 21, false, true),
            "── src/main.rs ──────\n"
        );
        assert_eq!(line("a", 10, false, false), "-- a -----\n");
    }

    #[test]
    fn dims_with_colours() {
        assert_eq!(line("a", 8, true, true), "\x1b[2m── a ───\x1b[22m\n");
    }

    #[test]
    fn sanitizes_and_shortens_names() {
        let out = line("evil\x1b]0;x\x07", 40, false, true);
        assert!(!out.contains('\x1b') && out.contains("^["));
        let out = line("some/very/long/path/to/file.rs", 20, false, true);
        assert_eq!(out.trim_end().chars().count(), 20);
        assert!(out.contains("…") && out.contains("file.rs"));
    }
}
