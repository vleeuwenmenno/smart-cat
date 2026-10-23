//! Active terminal capability detection.
//!
//! All queries go out in one batch on /dev/tty, followed by a DA1 request. Every
//! terminal answers DA1, and answers arrive in order, so anything not answered before
//! the DA1 reply is unsupported. A hard timeout covers terminals that answer nothing.

use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::time::{Duration, Instant};

use rustix::event::{PollFd, PollFlags, Timespec, poll};
use rustix::termios::{OptionalActions, Termios, tcgetattr, tcgetwinsize, tcsetattr};

pub const DEFAULT_TIMEOUT: Duration = Duration::from_millis(200);

#[derive(Debug, Clone, Default)]
pub struct Capabilities {
    /// The terminal answered DA1, so the other answers are trustworthy.
    pub answered: bool,
    pub kitty_graphics: bool,
    pub sixel: bool,
    pub da1: Vec<u16>,
    pub version: Option<String>,
    /// Cell size in pixels (width, height).
    pub cell_pixels: Option<(u16, u16)>,
    /// Text area size in pixels (width, height).
    pub window_pixels: Option<(u16, u16)>,
    pub background: Option<(u8, u8, u8)>,
    pub elapsed: Duration,
}

impl Capabilities {
    /// Relative luminance of the background, if known.
    pub fn background_is_light(&self) -> Option<bool> {
        self.background.map(|(r, g, b)| {
            let luma = 0.2126 * f64::from(r) + 0.7152 * f64::from(g) + 0.0722 * f64::from(b);
            luma > 128.0
        })
    }
}

/// Restores the terminal mode when dropped.
struct RawGuard<'a> {
    tty: &'a File,
    saved: Termios,
}

impl Drop for RawGuard<'_> {
    fn drop(&mut self) {
        let _ = tcsetattr(self.tty, OptionalActions::Now, &self.saved);
    }
}

/// Blocks the signals that would stop or kill scat while the terminal is raw, and
/// unblocks them when dropped. Signals that arrive in between stay pending and are
/// delivered on unblock, after the terminal mode has been restored.
struct SignalGuard {
    previous: libc::sigset_t,
}

impl SignalGuard {
    fn block() -> io::Result<Self> {
        // SAFETY: sigset_t is plain data that sigemptyset initialises; the pointers
        // passed to sigaddset and pthread_sigmask are valid for the calls.
        unsafe {
            let mut set: libc::sigset_t = std::mem::zeroed();
            libc::sigemptyset(&mut set);
            for signal in [
                libc::SIGINT,
                libc::SIGTERM,
                libc::SIGHUP,
                libc::SIGQUIT,
                libc::SIGTSTP,
            ] {
                libc::sigaddset(&mut set, signal);
            }
            let mut previous: libc::sigset_t = std::mem::zeroed();
            match libc::pthread_sigmask(libc::SIG_BLOCK, &set, &mut previous) {
                0 => Ok(Self { previous }),
                err => Err(io::Error::from_raw_os_error(err)),
            }
        }
    }
}

impl Drop for SignalGuard {
    fn drop(&mut self) {
        // SAFETY: restores the mask saved in `block`.
        unsafe {
            libc::pthread_sigmask(libc::SIG_SETMASK, &self.previous, std::ptr::null_mut());
        }
    }
}

pub fn query(timeout: Duration) -> io::Result<Capabilities> {
    let started = Instant::now();
    let tty = OpenOptions::new().read(true).write(true).open("/dev/tty")?;

    // Input already waiting (typeahead) would be mixed with the answers and consumed;
    // skip the query and let the caller fall back to environment hints.
    if rustix::io::ioctl_fionread(&tty).unwrap_or(0) > 0 {
        return Err(io::Error::other("input is pending on the terminal"));
    }

    // Declared before the raw guard, so it drops after it: signals are delivered only
    // once the terminal mode is back.
    let _signals = SignalGuard::block()?;
    let saved = tcgetattr(&tty)?;
    let mut raw = saved.clone();
    raw.make_raw();
    tcsetattr(&tty, OptionalActions::Now, &raw)?;
    let guard = RawGuard { tty: &tty, saved };

    let mut caps = Capabilities::default();
    if let Ok(size) = tcgetwinsize(&tty)
        && size.ws_xpixel > 0
        && size.ws_ypixel > 0
        && size.ws_col > 0
        && size.ws_row > 0
    {
        caps.window_pixels = Some((size.ws_xpixel, size.ws_ypixel));
        caps.cell_pixels = Some((size.ws_xpixel / size.ws_col, size.ws_ypixel / size.ws_row));
    }

    (&tty).write_all(&request())?;
    (&tty).flush()?;

    let mut buf = Vec::with_capacity(512);
    let mut chunk = [0u8; 256];
    loop {
        let remaining = timeout.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            break;
        }
        let timespec = Timespec {
            tv_sec: remaining.as_secs() as _,
            tv_nsec: remaining.subsec_nanos() as _,
        };
        let mut fds = [PollFd::new(&tty, PollFlags::IN)];
        match poll(&mut fds, Some(&timespec)) {
            Ok(0) => break,
            Ok(_) => {}
            Err(rustix::io::Errno::INTR) => continue,
            Err(err) => return Err(err.into()),
        }
        let n = (&tty).read(&mut chunk)?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
        if parse(&buf, &mut caps) {
            break;
        }
    }
    drop(guard);
    caps.elapsed = started.elapsed();
    Ok(caps)
}

fn request() -> Vec<u8> {
    let kitty = b"\x1b_Gi=31,s=1,v=1,a=q,t=d,f=24;AAAA\x1b\\";
    let mut out = Vec::new();
    if std::env::var_os("TMUX").is_some() {
        // tmux passthrough: wrap in DCS and double every ESC.
        out.extend_from_slice(b"\x1bPtmux;");
        for &b in kitty {
            if b == 0x1b {
                out.push(0x1b);
            }
            out.push(b);
        }
        out.extend_from_slice(b"\x1b\\");
    } else {
        out.extend_from_slice(kitty);
    }
    out.extend_from_slice(b"\x1b[>q"); // XTVERSION
    out.extend_from_slice(b"\x1b[16t"); // cell size in pixels
    out.extend_from_slice(b"\x1b[14t"); // text area size in pixels
    out.extend_from_slice(b"\x1b]11;?\x1b\\"); // background colour
    out.extend_from_slice(b"\x1b[c"); // DA1, always last
    out
}

/// Parses every complete reply in `buf`. Returns true once the DA1 reply is seen.
fn parse(buf: &[u8], caps: &mut Capabilities) -> bool {
    let mut i = 0;
    while i < buf.len() {
        if buf[i] != 0x1b || i + 1 >= buf.len() {
            i += 1;
            continue;
        }
        let rest = &buf[i + 2..];
        match buf[i + 1] {
            b'_' => {
                let Some(end) = find_st(rest) else {
                    return false;
                };
                let body = &rest[..end.0];
                if body.starts_with(b"G") && contains(body, b"i=31") && contains(body, b";OK") {
                    caps.kitty_graphics = true;
                }
                i += 2 + end.1;
            }
            b'P' => {
                let Some(end) = find_st(rest) else {
                    return false;
                };
                if let Some(version) = rest[..end.0].strip_prefix(b">|") {
                    caps.version = Some(String::from_utf8_lossy(version).into_owned());
                }
                i += 2 + end.1;
            }
            b']' => {
                let Some(end) = find_st(rest) else {
                    return false;
                };
                if let Some(color) = rest[..end.0].strip_prefix(b"11;") {
                    caps.background = parse_rgb(color);
                }
                i += 2 + end.1;
            }
            b'[' => {
                let Some(len) = rest.iter().position(|b| (0x40..=0x7e).contains(b)) else {
                    return false;
                };
                let params = &rest[..len];
                let final_byte = rest[len];
                i += 2 + len + 1;
                if final_byte == b'c' && params.first() == Some(&b'?') {
                    caps.da1 = numbers(&params[1..]);
                    caps.sixel = caps.da1.iter().skip(1).any(|&attr| attr == 4);
                    caps.answered = true;
                    return true;
                }
                if final_byte == b't' {
                    match numbers(params).as_slice() {
                        [6, h, w] => caps.cell_pixels = Some((*w, *h)),
                        [4, h, w] => caps.window_pixels = Some((*w, *h)),
                        _ => {}
                    }
                }
            }
            _ => i += 1,
        }
    }
    false
}

/// Finds a string terminator (ESC \ or BEL). Returns (body length, bytes consumed).
fn find_st(buf: &[u8]) -> Option<(usize, usize)> {
    for (idx, &b) in buf.iter().enumerate() {
        if b == 0x07 {
            return Some((idx, idx + 1));
        }
        if b == 0x1b && buf.get(idx + 1) == Some(&b'\\') {
            return Some((idx, idx + 2));
        }
    }
    None
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

fn numbers(params: &[u8]) -> Vec<u16> {
    String::from_utf8_lossy(params)
        .split(';')
        .filter_map(|p| p.parse().ok())
        .collect()
}

/// Parses `rgb:RRRR/GGGG/BBBB` with 1 to 4 hex digits per channel.
fn parse_rgb(spec: &[u8]) -> Option<(u8, u8, u8)> {
    let text = std::str::from_utf8(spec).ok()?.strip_prefix("rgb:")?;
    let mut channels = text.split('/').map(|c| {
        let value = u32::from_str_radix(c, 16).ok()?;
        let max = (1u32 << (4 * c.len() as u32)) - 1;
        (!c.is_empty() && c.len() <= 4).then(|| (value * 255 / max) as u8)
    });
    Some((channels.next()??, channels.next()??, channels.next()??))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_kitty_terminal_replies() {
        let reply = b"\x1b_Gi=31;OK\x1b\\\x1bP>|kitty(0.42.1)\x1b\\\x1b[6;20;10t\x1b[4;800;1200t\x1b]11;rgb:1e1e/1e1e/2e2e\x1b\\\x1b[?62;22;52c";
        let mut caps = Capabilities::default();
        assert!(parse(reply, &mut caps));
        assert!(caps.kitty_graphics);
        assert!(!caps.sixel);
        assert_eq!(caps.version.as_deref(), Some("kitty(0.42.1)"));
        assert_eq!(caps.cell_pixels, Some((10, 20)));
        assert_eq!(caps.window_pixels, Some((1200, 800)));
        assert_eq!(caps.background, Some((30, 30, 46)));
        assert_eq!(caps.background_is_light(), Some(false));
    }

    fn blocked(signal: libc::c_int) -> bool {
        // SAFETY: reads this thread's signal mask into a zeroed sigset_t.
        unsafe {
            let mut current: libc::sigset_t = std::mem::zeroed();
            libc::pthread_sigmask(libc::SIG_BLOCK, std::ptr::null(), &mut current);
            libc::sigismember(&current, signal) == 1
        }
    }

    #[test]
    fn signals_are_blocked_only_while_the_guard_lives() {
        assert!(!blocked(libc::SIGINT));
        let guard = SignalGuard::block().unwrap();
        for signal in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP, libc::SIGTSTP] {
            assert!(blocked(signal), "signal {signal}");
        }
        drop(guard);
        assert!(!blocked(libc::SIGINT));
        assert!(!blocked(libc::SIGTSTP));
    }

    #[test]
    fn parses_sixel_from_da1() {
        let mut caps = Capabilities::default();
        assert!(parse(b"\x1b]11;rgb:ff/ff/ff\x07\x1b[?62;4;22c", &mut caps));
        assert!(caps.sixel);
        assert!(!caps.kitty_graphics);
        assert_eq!(caps.background_is_light(), Some(true));
    }

    #[test]
    fn waits_for_incomplete_replies() {
        let mut caps = Capabilities::default();
        assert!(!parse(b"\x1b_Gi=31;OK\x1b\\\x1b[?62;4", &mut caps));
        assert!(caps.kitty_graphics);
        assert!(!caps.answered);
    }

    #[test]
    fn rgb_parsing_scales_channels() {
        assert_eq!(parse_rgb(b"rgb:ffff/0000/8080"), Some((255, 0, 128)));
        assert_eq!(parse_rgb(b"rgb:f/0/8"), Some((255, 0, 136)));
        assert_eq!(parse_rgb(b"rgb:zz/00/00"), None);
    }
}
