//! A short-lived cache of terminal query answers, so a run of `scat` commands in the
//! same terminal pays for the query (a few ms, or 200 ms on a terminal that never
//! answers) once every few minutes instead of every time.
//!
//! Entries live in `$XDG_RUNTIME_DIR/scat/term-<key>` (directory 0700, files 0600). No
//! `XDG_RUNTIME_DIR` means no cache. The key covers the controlling terminal device,
//! the session, `TERM`, `TERM_PROGRAM`, `KITTY_WINDOW_ID` and `TMUX_PANE`, so another
//! tab, a new login, ssh or another tmux pane gets its own entry. An entry is used for
//! [`TTL`]: the background colour changes when the user switches theme, and that is
//! picked up once the entry expires.
//!
//! The file is a few `key=value` lines. Anything unexpected (wrong owner, loose
//! permissions, a symlink, unparsable text) is treated as a miss.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::query::Capabilities;

/// How long an entry is used.
pub const TTL: Duration = Duration::from_secs(10 * 60);

const HEADER: &str = "scat-term-cache 1";

/// Identifies one terminal session: a hash of everything that changes what the
/// terminal answers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Key(u64);

/// The parts a [`Key`] is made of.
#[derive(Debug, Clone, Default)]
pub struct KeyParts {
    /// Device number of the controlling terminal.
    pub tty: u64,
    /// Session id (`getsid`).
    pub session: i32,
    pub term: Option<String>,
    pub term_program: Option<String>,
    pub kitty_window: Option<String>,
    pub tmux_pane: Option<String>,
}

impl Key {
    pub fn new(parts: &KeyParts) -> Self {
        // FNV-1a: stable across builds, unlike std's hasher.
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        let mut feed = |bytes: &[u8]| {
            for &b in bytes.iter().chain(&[0xff]) {
                hash ^= u64::from(b);
                hash = hash.wrapping_mul(0x0100_0000_01b3);
            }
        };
        feed(&parts.tty.to_le_bytes());
        feed(&parts.session.to_le_bytes());
        for value in [
            &parts.term,
            &parts.term_program,
            &parts.kitty_window,
            &parts.tmux_pane,
        ] {
            feed(value.as_deref().unwrap_or("\u{0}").as_bytes());
        }
        Key(hash)
    }

    /// The key for this process, or `None` without a controlling terminal.
    pub fn current() -> Option<Self> {
        let tty = controlling_tty()?;
        let session = rustix::process::getsid(None).ok()?.as_raw_nonzero().get();
        let var = |name| std::env::var(name).ok().filter(|v| !v.is_empty());
        Some(Self::new(&KeyParts {
            tty,
            session,
            term: var("TERM"),
            term_program: var("TERM_PROGRAM"),
            kitty_window: var("KITTY_WINDOW_ID"),
            tmux_pane: var("TMUX_PANE"),
        }))
    }

    fn file_name(&self) -> String {
        format!("term-{:016x}", self.0)
    }
}

/// The `tty_nr` field of /proc/self/stat: the controlling terminal's device number.
fn controlling_tty() -> Option<u64> {
    let stat = fs::read_to_string("/proc/self/stat").ok()?;
    // The command name (field 2) may contain spaces; fields after it are plain.
    let rest = &stat[stat.rfind(')')? + 1..];
    let tty: u64 = rest.split_whitespace().nth(4)?.parse().ok()?;
    (tty != 0).then_some(tty)
}

/// `$XDG_RUNTIME_DIR/scat`, or `None` when the variable is unset or not absolute.
pub fn dir() -> Option<PathBuf> {
    let base = PathBuf::from(std::env::var_os("XDG_RUNTIME_DIR")?);
    base.is_absolute().then(|| base.join("scat"))
}

/// A cached answer and how old it is.
#[derive(Debug, Clone)]
pub struct Entry {
    pub caps: Capabilities,
    pub age: Duration,
}

/// Only our own, private directories and files are trusted.
fn private(meta: &fs::Metadata) -> bool {
    meta.uid() == rustix::process::geteuid().as_raw() && meta.mode() & 0o077 == 0
}

/// Reads the entry for `key`, if one exists and is younger than [`TTL`].
pub fn load(dir: &Path, key: &Key, now: SystemTime) -> Option<Entry> {
    let dir_meta = fs::symlink_metadata(dir).ok()?;
    if !dir_meta.is_dir() || !private(&dir_meta) {
        return None;
    }
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(dir.join(key.file_name()))
        .ok()?;
    let meta = file.metadata().ok()?;
    if !meta.is_file() || !private(&meta) || meta.len() > 4096 {
        return None;
    }
    let mut text = String::new();
    file.take(4096).read_to_string(&mut text).ok()?;
    let (written, caps) = parse(&text)?;
    let now = now.duration_since(UNIX_EPOCH).ok()?.as_secs();
    // A timestamp from the future (clock changed) is not trusted either.
    let age = Duration::from_secs(now.checked_sub(written)?);
    (age < TTL).then_some(Entry { caps, age })
}

/// Writes the entry for `key`. Errors are for the caller to ignore: the cache is only
/// an optimisation.
pub fn store(dir: &Path, key: &Key, caps: &Capabilities, now: SystemTime) -> io::Result<()> {
    match fs::DirBuilder::new().mode(0o700).create(dir) {
        Ok(()) => {}
        Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {}
        Err(err) => return Err(err),
    }
    let meta = fs::symlink_metadata(dir)?;
    if !meta.is_dir() || !private(&meta) {
        return Err(io::Error::other("cache directory is not private"));
    }
    let written = now
        .duration_since(UNIX_EPOCH)
        .map_err(io::Error::other)?
        .as_secs();
    let target = dir.join(key.file_name());
    let temp = dir.join(format!("{}.{}.tmp", key.file_name(), std::process::id()));
    let result = (|| {
        let mut file: File = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&temp)?;
        file.write_all(format(written, caps).as_bytes())?;
        fs::rename(&temp, &target)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

fn format(written: u64, caps: &Capabilities) -> String {
    let flag = |b: bool| if b { 1 } else { 0 };
    let mut text = format!(
        "{HEADER}\ntime={written}\nanswered={}\nkitty={}\nsixel={}\n",
        flag(caps.answered),
        flag(caps.kitty_graphics),
        flag(caps.sixel)
    );
    if let Some((w, h)) = caps.cell_pixels {
        text.push_str(&format!("cell={w}x{h}\n"));
    }
    if let Some((r, g, b)) = caps.background {
        text.push_str(&format!("background={r:02x}{g:02x}{b:02x}\n"));
    }
    text
}

fn parse(text: &str) -> Option<(u64, Capabilities)> {
    let mut lines = text.lines();
    if lines.next()? != HEADER {
        return None;
    }
    let mut written = None;
    let mut caps = Capabilities::default();
    let flag = |v: &str| match v {
        "1" => Some(true),
        "0" => Some(false),
        _ => None,
    };
    for line in lines {
        let (name, value) = line.split_once('=')?;
        match name {
            "time" => written = Some(value.parse().ok()?),
            "answered" => caps.answered = flag(value)?,
            "kitty" => caps.kitty_graphics = flag(value)?,
            "sixel" => caps.sixel = flag(value)?,
            "cell" => {
                let (w, h) = value.split_once('x')?;
                caps.cell_pixels = Some((w.parse().ok()?, h.parse().ok()?));
            }
            "background" if value.len() == 6 => {
                let channel = |i: usize| u8::from_str_radix(value.get(i..i + 2)?, 16).ok();
                caps.background = Some((channel(0)?, channel(2)?, channel(4)?));
            }
            _ => return None,
        }
    }
    Some((written?, caps))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("scat-cache-test-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::DirBuilder::new().mode(0o700).create(&dir).unwrap();
        dir
    }

    fn sample() -> Capabilities {
        Capabilities {
            answered: true,
            kitty_graphics: true,
            sixel: false,
            cell_pixels: Some((10, 21)),
            background: Some((0x1e, 0x1e, 0x2e)),
            ..Default::default()
        }
    }

    fn key(session: i32) -> Key {
        Key::new(&KeyParts {
            tty: 0x8803,
            session,
            term: Some("xterm-kitty".into()),
            kitty_window: Some("3".into()),
            ..Default::default()
        })
    }

    #[test]
    fn roundtrip() {
        let runtime = temp_dir("roundtrip");
        let dir = runtime.join("scat");
        let now = SystemTime::now();
        store(&dir, &key(42), &sample(), now).unwrap();
        let mode = |p: &Path| fs::metadata(p).unwrap().mode() & 0o777;
        assert_eq!(mode(&dir), 0o700);
        assert_eq!(mode(&dir.join(key(42).file_name())), 0o600);

        let entry = load(&dir, &key(42), now + Duration::from_secs(5)).unwrap();
        assert_eq!(entry.age, Duration::from_secs(5));
        assert!(entry.caps.answered);
        assert!(entry.caps.kitty_graphics);
        assert!(!entry.caps.sixel);
        assert_eq!(entry.caps.cell_pixels, Some((10, 21)));
        assert_eq!(entry.caps.background, Some((0x1e, 0x1e, 0x2e)));

        // A terminal that did not answer is remembered too.
        let silent = Capabilities::default();
        store(&dir, &key(42), &silent, now).unwrap();
        let entry = load(&dir, &key(42), now).unwrap();
        assert!(!entry.caps.answered);
        assert_eq!(entry.caps.background, None);
        fs::remove_dir_all(&runtime).unwrap();
    }

    #[test]
    fn entries_expire() {
        let runtime = temp_dir("expiry");
        let dir = runtime.join("scat");
        let now = SystemTime::now();
        store(&dir, &key(1), &sample(), now).unwrap();
        assert!(load(&dir, &key(1), now + TTL - Duration::from_secs(1)).is_some());
        assert!(load(&dir, &key(1), now + TTL).is_none());
        // Written "in the future": the clock was changed, do not trust it.
        assert!(load(&dir, &key(1), now - Duration::from_secs(60)).is_none());
        fs::remove_dir_all(&runtime).unwrap();
    }

    #[test]
    fn key_changes_with_the_session_and_terminal() {
        assert_eq!(key(7), key(7));
        assert_ne!(key(7), key(8));
        let mut parts = KeyParts {
            tty: 1,
            session: 7,
            ..Default::default()
        };
        let base = Key::new(&parts);
        parts.tmux_pane = Some("%1".into());
        assert_ne!(Key::new(&parts), base);
        parts.tmux_pane = None;
        parts.tty = 2;
        assert_ne!(Key::new(&parts), base);
        // Fields cannot run into each other.
        let a = Key::new(&KeyParts {
            term: Some("ab".into()),
            term_program: Some("c".into()),
            ..Default::default()
        });
        let b = Key::new(&KeyParts {
            term: Some("a".into()),
            term_program: Some("bc".into()),
            ..Default::default()
        });
        assert_ne!(a, b);
    }

    #[test]
    fn untrusted_entries_are_ignored() {
        let runtime = temp_dir("untrusted");
        let dir = runtime.join("scat");
        let now = SystemTime::now();
        store(&dir, &key(1), &sample(), now).unwrap();
        let path = dir.join(key(1).file_name());

        fs::set_permissions(&path, std::os::unix::fs::PermissionsExt::from_mode(0o644)).unwrap();
        assert!(load(&dir, &key(1), now).is_none());

        fs::remove_file(&path).unwrap();
        fs::write(&path, "garbage").unwrap();
        fs::set_permissions(&path, std::os::unix::fs::PermissionsExt::from_mode(0o600)).unwrap();
        assert!(load(&dir, &key(1), now).is_none());

        // A symlink in place of the file is not followed.
        fs::remove_file(&path).unwrap();
        let elsewhere = runtime.join("elsewhere");
        fs::write(&elsewhere, format(0, &sample())).unwrap();
        std::os::unix::fs::symlink(&elsewhere, &path).unwrap();
        assert!(load(&dir, &key(1), now).is_none());
        fs::remove_dir_all(&runtime).unwrap();
    }

    #[test]
    fn parse_rejects_unknown_lines() {
        assert!(parse("scat-term-cache 1\ntime=5\nanswered=1\n").is_some());
        assert!(parse("scat-term-cache 1\ntime=5\nversion=\x1b]0;x\n").is_none());
        assert!(parse("scat-term-cache 2\ntime=5\n").is_none());
        assert!(parse("scat-term-cache 1\nanswered=1\n").is_none());
    }
}
