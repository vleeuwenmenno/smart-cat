//! kitty graphics protocol output: direct transmission, base64 in chunks.
//!
//! Spec: <https://sw.kovidgoyal.net/kitty/graphics-protocol/>. The payload is a PNG
//! (`f=100`), sent in the escape sequence itself (`t=d`), so it works over SSH.

use std::io::{self, Write};

/// Maximum base64 bytes per escape sequence, as the spec requires.
pub const CHUNK: usize = 4096;

/// Where and how to place the image.
#[derive(Debug, Clone, Copy, Default)]
pub struct Placement {
    /// Width in cells; kitty computes the height from the aspect ratio.
    pub cols: u16,
    /// Height in cells; 0 lets kitty compute it. Set when text is laid out beside
    /// the image, so the image covers exactly the rows reserved for it.
    pub rows: u16,
    /// Image and placement ids, to replace the previous frame of an animation.
    pub ids: Option<(u32, u32)>,
    /// Leave the cursor where it is (`C=1`), for redrawing in place.
    pub keep_cursor: bool,
}

/// Writes one image as `a=T` (transmit and display) escape sequences. `png` must be a
/// complete PNG file. Replies are suppressed (`q=2`).
pub fn write_png<W: Write>(out: &mut W, png: &[u8], placement: Placement) -> io::Result<()> {
    let data = base64(png);
    let mut chunks = data.chunks(CHUNK).peekable();
    let mut first = true;
    // An empty payload still needs one (empty) chunk.
    if chunks.peek().is_none() {
        return Ok(());
    }
    while let Some(chunk) = chunks.next() {
        let more = u8::from(chunks.peek().is_some());
        out.write_all(b"\x1b_G")?;
        if first {
            write!(out, "a=T,f=100,t=d,q=2")?;
            if placement.cols > 0 {
                write!(out, ",c={}", placement.cols)?;
            }
            if placement.rows > 0 {
                write!(out, ",r={}", placement.rows)?;
            }
            if let Some((image, place)) = placement.ids {
                write!(out, ",i={image},p={place}")?;
            }
            if placement.keep_cursor {
                out.write_all(b",C=1")?;
            }
            write!(out, ",m={more}")?;
            first = false;
        } else {
            write!(out, "q=2,m={more}")?;
        }
        out.write_all(b";")?;
        out.write_all(chunk)?;
        out.write_all(b"\x1b\\")?;
    }
    Ok(())
}

/// Standard base64 with padding.
pub fn base64(data: &[u8]) -> Vec<u8> {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = Vec::with_capacity(data.len().div_ceil(3) * 4);
    for group in data.chunks(3) {
        let b = [
            group[0],
            group.get(1).copied().unwrap_or(0),
            group.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(TABLE[(n >> 18) as usize & 63]);
        out.push(TABLE[(n >> 12) as usize & 63]);
        out.push(if group.len() > 1 {
            TABLE[(n >> 6) as usize & 63]
        } else {
            b'='
        });
        out.push(if group.len() > 2 {
            TABLE[n as usize & 63]
        } else {
            b'='
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_rfc4648() {
        assert_eq!(base64(b""), b"");
        assert_eq!(base64(b"f"), b"Zg==");
        assert_eq!(base64(b"fo"), b"Zm8=");
        assert_eq!(base64(b"foo"), b"Zm9v");
        assert_eq!(base64(b"foobar"), b"Zm9vYmFy");
    }

    fn sequences(out: &[u8]) -> Vec<&[u8]> {
        let text = out;
        let mut seqs = Vec::new();
        let mut rest = text;
        while !rest.is_empty() {
            assert!(
                rest.starts_with(b"\x1b_G"),
                "sequence must start with APC G"
            );
            let end = rest.windows(2).position(|w| w == b"\x1b\\").unwrap();
            seqs.push(&rest[3..end]);
            rest = &rest[end + 2..];
        }
        seqs
    }

    #[test]
    fn small_image_is_one_sequence() {
        let mut out = Vec::new();
        let placement = Placement {
            cols: 12,
            ..Default::default()
        };
        write_png(&mut out, b"abc", placement).unwrap();
        let seqs = sequences(&out);
        assert_eq!(seqs.len(), 1);
        assert_eq!(seqs[0], b"a=T,f=100,t=d,q=2,c=12,m=0;YWJj");
    }

    #[test]
    fn large_payloads_are_chunked() {
        let data = vec![7u8; 10_000]; // 13336 base64 bytes: 4 chunks
        let mut out = Vec::new();
        let placement = Placement {
            cols: 5,
            rows: 3,
            ids: Some((9, 1)),
            keep_cursor: true,
        };
        write_png(&mut out, &data, placement).unwrap();
        let seqs = sequences(&out);
        assert_eq!(seqs.len(), 4);
        assert!(seqs[0].starts_with(b"a=T,f=100,t=d,q=2,c=5,r=3,i=9,p=1,C=1,m=1;"));
        assert!(seqs[1].starts_with(b"q=2,m=1;"));
        assert!(seqs[3].starts_with(b"q=2,m=0;"));
        let mut payload = Vec::new();
        for seq in &seqs {
            let body = &seq[seq.iter().position(|&b| b == b';').unwrap() + 1..];
            assert!(body.len() <= CHUNK);
            assert!(
                body.iter()
                    .all(|b| b.is_ascii_alphanumeric() || b"+/=".contains(b))
            );
            payload.extend_from_slice(body);
        }
        assert_eq!(payload, base64(&data));
    }
}
