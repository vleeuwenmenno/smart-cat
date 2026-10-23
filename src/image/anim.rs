//! Playing an animation once, redrawn in place.
//!
//! Every frame is decoded, resized and encoded just before it is shown, wrapped in
//! synchronized output (`CSI ? 2026 h/l`) with the cursor hidden. kitty gets each
//! frame as a replacement of the same image id and placement, so no terminal-side
//! animation state is left behind. Playback stops after one loop or `MAX_PLAYBACK`;
//! the last frame stays on screen. Ctrl-C restores the cursor, then exits as SIGINT.

use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use image::DynamicImage;

use super::decode::Decoded;
use super::{FrameMode, Layout, Screen, encode};

const MIN_DELAY: Duration = Duration::from_millis(20);
/// A runaway animation still ends: cat must terminate.
const MAX_PLAYBACK: Duration = Duration::from_secs(60);

static INTERRUPTED: AtomicBool = AtomicBool::new(false);

extern "C" fn on_sigint(_: libc::c_int) {
    INTERRUPTED.store(true, Ordering::SeqCst);
}

/// Installs the SIGINT flag handler and restores the previous one when dropped.
struct SigintGuard {
    previous: libc::sighandler_t,
}

impl SigintGuard {
    fn install() -> Self {
        INTERRUPTED.store(false, Ordering::SeqCst);
        // SAFETY: the handler only stores to an atomic, which is async-signal-safe.
        let previous =
            unsafe { libc::signal(libc::SIGINT, on_sigint as *const () as libc::sighandler_t) };
        Self { previous }
    }
}

impl Drop for SigintGuard {
    fn drop(&mut self) {
        // SAFETY: restores the disposition saved in `install`.
        unsafe {
            libc::signal(libc::SIGINT, self.previous);
        }
    }
}

/// Plays `decoded` if it is animated. Returns the number of frames shown, or `None`
/// for a still image (nothing was written).
pub(super) fn play<W: Write>(
    out: &mut W,
    screen: &Screen,
    layout: &Layout,
    decoded: &Decoded,
) -> io::Result<Option<usize>> {
    let Ok(Some(mut frames)) = decoded.frames() else {
        return Ok(None);
    };
    let Some(Ok(first)) = frames.next() else {
        return Ok(None);
    };
    let second = match frames.next() {
        Some(Ok(frame)) => frame,
        _ => return Ok(None),
    };

    let rows = layout.rows;
    let mode = FrameMode::Animated {
        image_id: 0x5ca7_0000 | (std::process::id() & 0xffff),
    };
    let guard = SigintGuard::install();
    // Reserve the rows (scrolling now, not mid-animation), go back up, remember the spot.
    out.write_all(b"\x1b[?25l")?;
    out.write_all(&b"\n".repeat(usize::from(rows)))?;
    write!(out, "\x1b[{rows}A\r\x1b7")?;

    let started = Instant::now();
    let mut shown = 0;
    let mut result = Ok(());
    for frame in [Ok(first), Ok(second)].into_iter().chain(frames) {
        let Ok(frame) = frame else { break };
        let frame_start = Instant::now();
        let (num, den) = frame.delay().numer_denom_ms();
        let delay = Duration::from_millis(u64::from(num / den.max(1))).max(MIN_DELAY);
        let img = DynamicImage::ImageRgba8(frame.into_buffer());
        let step = encode(screen, layout, &img, None, mode).and_then(|bytes| {
            out.write_all(b"\x1b[?2026h\x1b8")?;
            out.write_all(&bytes)?;
            out.write_all(b"\x1b[?2026l")?;
            out.flush()
        });
        if let Err(err) = step {
            result = Err(err);
            break;
        }
        shown += 1;
        if !wait_until(frame_start + delay) || started.elapsed() > MAX_PLAYBACK {
            break;
        }
    }

    // Below the image, cursor back on.
    let _ = write!(out, "\x1b8\x1b[{rows}B\r\x1b[?25h");
    let _ = out.flush();
    drop(guard);
    if INTERRUPTED.load(Ordering::SeqCst) {
        // SAFETY: plain libc calls; re-raise so the shell sees the usual Ctrl-C status.
        unsafe {
            libc::signal(libc::SIGINT, libc::SIG_DFL);
            libc::raise(libc::SIGINT);
        }
    }
    result.map(|()| Some(shown))
}

/// Sleeps until `deadline` in short steps. Returns false when interrupted.
fn wait_until(deadline: Instant) -> bool {
    loop {
        if INTERRUPTED.load(Ordering::SeqCst) {
            return false;
        }
        let now = Instant::now();
        if now >= deadline {
            return true;
        }
        std::thread::sleep((deadline - now).min(Duration::from_millis(10)));
    }
}
