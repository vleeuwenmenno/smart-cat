//! File type detection from the first bytes of a file.

/// How many leading bytes are inspected.
pub const SNIFF_BYTES: usize = 8 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    Text,
    Binary,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Detected {
    pub kind: Kind,
    pub mime: String,
}

pub fn detect(head: &[u8]) -> Detected {
    let head = &head[..head.len().min(SNIFF_BYTES)];
    let inferred = infer::get(head).map(|t| t.mime_type());
    let textual_mime = inferred.is_none_or(|m| m.starts_with("text/"));

    if textual_mime && looks_like_text(head) {
        return Detected {
            kind: Kind::Text,
            mime: inferred.unwrap_or("text/plain").to_string(),
        };
    }
    Detected {
        kind: Kind::Binary,
        mime: inferred.unwrap_or("application/octet-stream").to_string(),
    }
}

/// Text if there are no NUL bytes and at most a few invalid UTF-8 sequences
/// (latin-1 files and a multi-byte character cut off at the sniff boundary are fine).
fn looks_like_text(head: &[u8]) -> bool {
    if head.contains(&0) {
        return false;
    }
    let mut invalid = 0usize;
    let mut rest = head;
    while !rest.is_empty() {
        match std::str::from_utf8(rest) {
            Ok(_) => break,
            Err(err) => {
                let skip =
                    err.valid_up_to() + err.error_len().unwrap_or(rest.len() - err.valid_up_to());
                if err.error_len().is_some() {
                    invalid += 1;
                }
                rest = &rest[skip..];
            }
        }
    }
    invalid * 100 <= head.len().max(1) * 5
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_text() {
        assert_eq!(detect(b"fn main() {}\n").kind, Kind::Text);
        assert_eq!(detect(b"").kind, Kind::Text);
        assert_eq!(detect("caf\u{e9}\n".as_bytes()).kind, Kind::Text);
    }

    #[test]
    fn detects_binary() {
        let png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR";
        let d = detect(png);
        assert_eq!(d.kind, Kind::Binary);
        assert_eq!(d.mime, "image/png");
        assert_eq!(detect(b"ab\0cd").kind, Kind::Binary);
    }

    #[test]
    fn truncated_utf8_at_boundary_is_text() {
        let mut data = "x".repeat(100).into_bytes();
        data.extend_from_slice(&"\u{e9}".as_bytes()[..1]);
        assert_eq!(detect(&data).kind, Kind::Text);
    }
}
