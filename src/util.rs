use std::ffi::OsStr;
use std::io::{self, BufRead, Write};
use std::sync::OnceLock;

/// Prints a message on standard error with the program name in front, like coreutils:
/// `scat: NAME: message`, or `cat: ...` when scat was started under the name `cat`.
///
/// Messages often carry text from a file (a part name, a table name, a decoder error),
/// so the whole message is sanitized: control characters and escape sequences are
/// shown as `^X`, never sent to the terminal.
#[macro_export]
macro_rules! report {
    ($($arg:tt)*) => {
        eprintln!(
            "{}: {}",
            $crate::util::prog_name(),
            $crate::util::sanitize_message(&format!($($arg)*))
        )
    };
}

/// Makes a message safe for the terminal (see [`report!`]). Line breaks are kept, so a
/// multi-line message stays readable; every other control character becomes `^X`.
/// Text that is already sanitized comes out unchanged.
pub fn sanitize_message(message: &str) -> String {
    message
        .split('\n')
        .map(crate::render::sanitize_str)
        .collect::<Vec<_>>()
        .join("\n")
}

/// Whether a program name (`argv[0]`) is `cat`, after stripping any directory.
pub fn is_cat_name(argv0: &OsStr) -> bool {
    std::path::Path::new(argv0).file_name() == Some(OsStr::new("cat"))
}

/// True when scat was started under the name `cat` (a `cat -> scat` symlink).
pub fn invoked_as_cat() -> bool {
    static INVOKED: OnceLock<bool> = OnceLock::new();
    *INVOKED.get_or_init(|| std::env::args_os().next().is_some_and(|a| is_cat_name(&a)))
}

/// The name used in messages: `cat` when invoked as cat, otherwise `scat`.
pub fn prog_name() -> &'static str {
    if invoked_as_cat() { "cat" } else { "scat" }
}

/// Formats an I/O error the way coreutils does: the strerror text without Rust's
/// " (os error N)" suffix.
pub fn io_message(err: &io::Error) -> String {
    let text = err.to_string();
    match text.find(" (os error ") {
        Some(idx) => text[..idx].to_string(),
        None => text,
    }
}

/// Reads one line into `buf`, including its trailing newline.
///
/// Stops early once `buf` holds `max` bytes, so a file without newlines (or `/dev/zero`)
/// never grows the buffer without bound. Returns `None` at end of input, otherwise
/// whether the line ended with a newline.
pub fn read_line_capped<R: BufRead>(
    reader: &mut R,
    buf: &mut Vec<u8>,
    max: usize,
) -> io::Result<Option<bool>> {
    buf.clear();
    loop {
        let available = match reader.fill_buf() {
            Ok(bytes) => bytes,
            Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
            Err(err) => return Err(err),
        };
        if available.is_empty() {
            return Ok(if buf.is_empty() { None } else { Some(false) });
        }
        let take = available.len().min(max - buf.len());
        if let Some(pos) = available[..take].iter().position(|&b| b == b'\n') {
            buf.extend_from_slice(&available[..=pos]);
            reader.consume(pos + 1);
            return Ok(Some(true));
        }
        buf.extend_from_slice(&available[..take]);
        reader.consume(take);
        if buf.len() >= max {
            return Ok(Some(false));
        }
    }
}

/// A writer that remembers whether a write failed, so callers can tell output errors
/// (fatal) apart from errors reading an input file (report and continue).
pub struct TrackedWriter<W: Write> {
    inner: W,
    pub failed: bool,
}

impl<W: Write> TrackedWriter<W> {
    pub fn new(inner: W) -> Self {
        Self {
            inner,
            failed: false,
        }
    }
}

impl<W: Write> Write for TrackedWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.inner.write(buf).inspect_err(|_| self.failed = true)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush().inspect_err(|_| self.failed = true)
    }
}

/// Looks up an executable on `PATH`.
pub fn which(name: &str) -> Option<std::path::PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .filter(|dir| dir.is_absolute())
        .map(|dir| dir.join(name))
        .find(|candidate| {
            candidate
                .metadata()
                .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
                .unwrap_or(false)
        })
}

/// Human readable byte size, e.g. "1.5 MiB".
pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_os_error_suffix() {
        let err = io::Error::from_raw_os_error(libc::ENOENT);
        assert_eq!(io_message(&err), "No such file or directory");
    }

    #[test]
    fn caps_long_lines() {
        let mut reader = io::Cursor::new(b"abcdef\ngh".to_vec());
        let mut buf = Vec::new();
        assert_eq!(
            read_line_capped(&mut reader, &mut buf, 4).unwrap(),
            Some(false)
        );
        assert_eq!(buf, b"abcd");
        assert_eq!(
            read_line_capped(&mut reader, &mut buf, 4).unwrap(),
            Some(true)
        );
        assert_eq!(buf, b"ef\n");
        assert_eq!(
            read_line_capped(&mut reader, &mut buf, 4).unwrap(),
            Some(false)
        );
        assert_eq!(buf, b"gh");
        assert_eq!(read_line_capped(&mut reader, &mut buf, 4).unwrap(), None);
    }

    #[test]
    fn recognises_cat_names() {
        assert!(is_cat_name(OsStr::new("cat")));
        assert!(is_cat_name(OsStr::new("/usr/local/bin/cat")));
        assert!(is_cat_name(OsStr::new("./cat")));
        assert!(!is_cat_name(OsStr::new("scat")));
        assert!(!is_cat_name(OsStr::new("/bin/cat2")));
        assert!(!is_cat_name(OsStr::new("")));
    }

    #[test]
    fn messages_are_sanitized_once() {
        let message = "missing ppt/slides/\x1b]0;title\x07.xml\nsecond\tline";
        let clean = sanitize_message(message);
        assert!(
            !clean.contains('\x1b') && !clean.contains('\x07'),
            "{clean:?}"
        );
        assert_eq!(clean, "missing ppt/slides/^[]0;title^G.xml\nsecond line");
        assert_eq!(sanitize_message(&clean), clean);
    }

    #[test]
    fn human_sizes() {
        assert_eq!(human_size(12), "12 B");
        assert_eq!(human_size(1536), "1.5 KiB");
    }
}
