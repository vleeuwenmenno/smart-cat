//! Transparent decompression of single-stream gzip, xz, zstd and bzip2 files.
//!
//! All four decoders are pure Rust, so the static musl build needs no system library.
//! Only used when rendering; piped output is never decompressed.

use std::cell::Cell;
use std::io::{self, Cursor, Read};
use std::rc::Rc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Gzip,
    Xz,
    Zstd,
    Bzip2,
}

/// The compression format named by the magic bytes at the start of `head`.
pub fn detect(head: &[u8]) -> Option<Format> {
    if head.starts_with(&[0x1f, 0x8b, 0x08]) {
        Some(Format::Gzip)
    } else if head.starts_with(&[0xfd, b'7', b'z', b'X', b'Z', 0]) {
        Some(Format::Xz)
    } else if head.starts_with(&[0x28, 0xb5, 0x2f, 0xfd]) {
        Some(Format::Zstd)
    } else if head.starts_with(b"BZh") && head.get(3).is_some_and(|b| (b'1'..=b'9').contains(b)) {
        Some(Format::Bzip2)
    } else {
        None
    }
}

/// True for a compressed tar archive: those are not text streams. Decided by name
/// (`.tar.gz`, `.tgz`, ...) and by the `ustar` magic of the first decompressed block.
pub fn is_tarball(file_name: Option<&str>, head: &[u8], format: Format) -> bool {
    if let Some(name) = file_name {
        let lower = name.to_ascii_lowercase();
        let stem = strip_suffix(&lower).unwrap_or(&lower);
        const SHORT: [&str; 6] = [".tgz", ".txz", ".tzst", ".tbz", ".tbz2", ".taz"];
        if stem.ends_with(".tar") || SHORT.iter().any(|s| lower.ends_with(s)) {
            return true;
        }
    }
    let Ok(mut reader) = decoder(format, Box::new(Cursor::new(head.to_vec()))) else {
        return false;
    };
    let mut block = [0u8; 512];
    let mut filled = 0;
    while filled < block.len() {
        match reader.read(&mut block[filled..]) {
            Ok(0) | Err(_) => break,
            Ok(n) => filled += n,
        }
    }
    filled >= 262 && &block[257..262] == b"ustar"
}

fn strip_suffix(name: &str) -> Option<&str> {
    [".gz", ".xz", ".zst", ".zstd", ".bz2", ".gzip"]
        .iter()
        .find_map(|s| name.strip_suffix(s))
}

/// The name of the file inside: `app.log.gz` is `app.log`. Other names are kept.
pub fn inner_name(name: &str) -> String {
    let lower = name.to_ascii_lowercase();
    match strip_suffix(&lower) {
        Some(stem) if !stem.is_empty() => name[..stem.len()].to_string(),
        _ => name.to_string(),
    }
}

fn decoder(format: Format, source: Box<dyn Read>) -> io::Result<Box<dyn Read>> {
    Ok(match format {
        Format::Gzip => Box::new(flate2::read::MultiGzDecoder::new(source)),
        // 256 MiB of dictionary memory per block is plenty; bigger fails cleanly.
        Format::Xz => Box::new(lzma_rust2::XzReader::new_mem_limit(
            source,
            true,
            256 * 1024,
        )),
        Format::Zstd => {
            Box::new(ruzstd::decoding::StreamingDecoder::new(source).map_err(io::Error::other)?)
        }
        Format::Bzip2 => Box::new(bzip2::read::MultiBzDecoder::new(source)),
    })
}

/// Ends the stream after `left` bytes and records that it did so.
struct Capped {
    inner: Box<dyn Read>,
    left: u64,
    hit: Rc<Cell<bool>>,
}

impl Read for Capped {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.hit.get() || buf.is_empty() {
            return Ok(0);
        }
        if self.left == 0 {
            // Exactly at the limit: only a truncation if more data follows.
            let mut probe = [0u8; 1];
            if self.inner.read(&mut probe)? > 0 {
                self.hit.set(true);
            }
            return Ok(0);
        }
        let max = buf
            .len()
            .min(usize::try_from(self.left).unwrap_or(usize::MAX));
        let n = self.inner.read(&mut buf[..max])?;
        self.left -= n as u64;
        Ok(n)
    }
}

/// A decoded stream and the flag that tells whether the size cap cut it off.
pub type Decoded = (Box<dyn Read>, Rc<Cell<bool>>);

/// A streaming decoder over `source`, cut off after `limit` decompressed bytes (a
/// guard against decompression bombs). The flag is set when output was cut off.
pub fn open(format: Format, source: Box<dyn Read>, limit: u64) -> io::Result<Decoded> {
    let hit = Rc::new(Cell::new(false));
    let reader = Capped {
        inner: decoder(format, source)?,
        left: limit,
        hit: hit.clone(),
    };
    Ok((Box::new(reader), hit))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_magic() {
        assert_eq!(detect(&[0x1f, 0x8b, 8, 0]), Some(Format::Gzip));
        assert_eq!(detect(b"BZh91AY"), Some(Format::Bzip2));
        assert_eq!(detect(b"BZh"), None);
        assert_eq!(detect(b"plain"), None);
    }

    #[test]
    fn names_inside() {
        assert_eq!(inner_name("app.log.gz"), "app.log");
        assert_eq!(inner_name("APP.LOG.GZ"), "APP.LOG");
        assert_eq!(inner_name("data"), "data");
        assert_eq!(inner_name(".gz"), ".gz");
    }

    #[test]
    fn tar_names() {
        assert!(is_tarball(Some("a.tar.gz"), b"", Format::Gzip));
        assert!(is_tarball(Some("a.TGZ"), b"", Format::Gzip));
        assert!(!is_tarball(Some("a.log.gz"), b"", Format::Gzip));
    }
}
