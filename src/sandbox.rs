//! Running untrusted parsers in a separate process.
//!
//! PDFs and spreadsheets are untrusted input, and their parsers (hayro, calamine) can
//! panic, loop or allocate without bound. Release builds use `panic = "abort"`, so a
//! panic in a parser would kill scat itself. Those parsers therefore run in a child
//! process: scat re-runs its own binary with a hidden first argument
//! ([`PDF_FLAG`], [`SHEET_FLAG`]), and the poppler tools run the same way. Every child
//! gets:
//!
//! - an address space limit (`RLIMIT_AS`, [`MEMORY_LIMIT`] unless set) and no core
//!   dumps,
//! - a CPU time limit and a wall clock timeout ([`TIMEOUT`] unless set), after which it
//!   is killed,
//! - a cap on how much output is read back,
//! - `/dev/null` or the given bytes (in an anonymous memory file) as standard input,
//!   and argv-only arguments.
//!
//! A crash, timeout or error in the child becomes an error message; scat goes on.

use std::ffi::OsStr;
use std::io::{self, Read};
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Hidden first argument that turns scat into the PDF helper process.
pub const PDF_FLAG: &str = "--internal-pdf";

/// Hidden first argument that turns scat into the spreadsheet helper process.
pub const SHEET_FLAG: &str = "--internal-sheet";

/// Wall clock limit for one helper run.
pub const TIMEOUT: Duration = Duration::from_secs(10);

/// Address space limit for helpers.
pub const MEMORY_LIMIT: u64 = 1 << 30;

/// CPU seconds a helper gets on top of its wall clock limit before the kernel stops it
/// (a backstop for the timeout).
const CPU_GRACE: u64 = 5;

/// Most stdout read back from a helper (a PNG of a page fitted to the terminal is a
/// few MiB at most).
pub const MAX_OUTPUT: u64 = 128 << 20;

#[derive(Debug)]
pub enum RunError {
    /// The program could not be started.
    Spawn(io::Error),
    TimedOut(Duration),
    /// Killed by a signal (a panic aborts, the memory limit ends in SIGABRT or SIGSEGV).
    Crashed(i32),
    /// Non-zero exit; the first line of its stderr.
    Failed(String),
}

impl std::fmt::Display for RunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RunError::Spawn(err) => {
                write!(f, "cannot start helper: {}", crate::util::io_message(err))
            }
            RunError::TimedOut(limit) => write!(f, "gave up after {} s", limit.as_secs()),
            RunError::Crashed(signal) => write!(f, "the parser crashed (signal {signal})"),
            RunError::Failed(message) if message.is_empty() => f.write_str("the parser failed"),
            RunError::Failed(message) => f.write_str(message),
        }
    }
}

/// A command that runs scat's own helper `flag` ([`PDF_FLAG`], [`SHEET_FLAG`]) with
/// `args`.
pub fn helper<I, S>(flag: &str, args: I) -> Command
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    // /proc/self/exe still works when the binary was replaced or deleted since start.
    let exe = if std::path::Path::new("/proc/self/exe").exists() {
        std::path::PathBuf::from("/proc/self/exe")
    } else {
        std::env::current_exe().unwrap_or_else(|_| "scat".into())
    };
    let mut cmd = Command::new(exe);
    cmd.arg(flag).args(args);
    cmd
}

/// Limits and input for one [`run_with`].
#[derive(Debug, Clone, Copy)]
pub struct Limits<'a> {
    pub timeout: Duration,
    pub memory: u64,
    /// Written to the child's standard input; `None` gives it `/dev/null`.
    pub stdin: Option<&'a [u8]>,
}

impl Default for Limits<'_> {
    fn default() -> Self {
        Limits {
            timeout: TIMEOUT,
            memory: MEMORY_LIMIT,
            stdin: None,
        }
    }
}

/// Runs `cmd` with the limits described in the module docs and returns its stdout.
pub fn run(cmd: Command) -> Result<Vec<u8>, RunError> {
    run_within(cmd, TIMEOUT)
}

/// [`run`] with its own wall clock limit.
pub fn run_within(cmd: Command, timeout: Duration) -> Result<Vec<u8>, RunError> {
    run_with(
        cmd,
        Limits {
            timeout,
            ..Limits::default()
        },
    )
}

/// Runs `cmd` with `limits` and returns its stdout.
pub fn run_with(mut cmd: Command, limits: Limits) -> Result<Vec<u8>, RunError> {
    let Limits {
        timeout,
        memory,
        stdin,
    } = limits;
    // The input goes through an anonymous memory file, not a pipe: scat keeps SIGPIPE
    // at its default, so writing to a child that died early would kill scat too.
    cmd.stdin(match stdin {
        Some(data) => Stdio::from(memory_file(data).map_err(RunError::Spawn)?),
        None => Stdio::null(),
    })
    .stdout(Stdio::piped())
    .stderr(Stdio::piped());
    let cpu = timeout.as_secs() + CPU_GRACE;
    // SAFETY: only setrlimit, which is async-signal-safe, runs between fork and exec.
    unsafe {
        cmd.pre_exec(move || {
            limit(libc::RLIMIT_AS, memory)?;
            limit(libc::RLIMIT_CPU, cpu)?;
            limit(libc::RLIMIT_CORE, 0)?;
            Ok(())
        });
    }
    let child = cmd.spawn().map_err(RunError::Spawn)?;
    wait(child, timeout)
}

/// `data` in an anonymous memory file, positioned at its start.
fn memory_file(data: &[u8]) -> io::Result<std::fs::File> {
    use std::io::{Seek, Write};
    let fd = rustix::fs::memfd_create("scat-input", rustix::fs::MemfdFlags::CLOEXEC)?;
    let mut file = std::fs::File::from(fd);
    file.write_all(data)?;
    file.rewind()?;
    Ok(file)
}

/// Collects the output of `child` and waits for it, at most `timeout`.
fn wait(mut child: std::process::Child, timeout: Duration) -> Result<Vec<u8>, RunError> {
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let out_reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(stdout) = stdout {
            let _ = stdout.take(MAX_OUTPUT).read_to_end(&mut buf);
        }
        buf
    });
    let err_reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(stderr) = stderr {
            let _ = stderr.take(64 * 1024).read_to_end(&mut buf);
        }
        buf
    });

    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = out_reader.join();
                let _ = err_reader.join();
                return Err(RunError::TimedOut(timeout));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(2)),
            Err(err) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(RunError::Spawn(err));
            }
        }
    };
    let stdout = out_reader.join().unwrap_or_default();
    let stderr = err_reader.join().unwrap_or_default();
    if status.success() {
        return Ok(stdout);
    }
    let text = String::from_utf8_lossy(&stderr);
    let line = text.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
    let line = line
        .strip_prefix("scat: ")
        .or_else(|| line.strip_prefix("cat: "))
        .unwrap_or(line);
    match status.signal() {
        // A helper that said why before it died (see `report_panics`).
        Some(_) if line.starts_with("the ") && line.ends_with(" crashed") => {
            Err(RunError::Failed(line.chars().take(200).collect()))
        }
        Some(signal) => Err(RunError::Crashed(signal)),
        None => Err(RunError::Failed(line.chars().take(200).collect())),
    }
}

/// In a helper: report a panic as one `scat: the PARSER crashed` line on stderr, which
/// the parent shows instead of the signal (a release build aborts right after) or a
/// panic message with source locations (a debug build).
pub fn report_panics(parser: &'static str) {
    std::panic::set_hook(Box::new(move |_| eprintln!("scat: the {parser} crashed")));
}

#[cfg(target_env = "gnu")]
type Resource = libc::__rlimit_resource_t;
#[cfg(not(target_env = "gnu"))]
type Resource = libc::c_int;

fn limit(resource: Resource, value: u64) -> io::Result<()> {
    let lim = libc::rlimit {
        rlim_cur: value as libc::rlim_t,
        rlim_max: value as libc::rlim_t,
    };
    // SAFETY: plain syscall with a valid pointer to a local struct.
    if unsafe { libc::setrlimit(resource, &lim) } == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sh(script: &str) -> Command {
        let mut cmd = Command::new("sh");
        cmd.args(["-c", script]);
        cmd
    }

    #[test]
    fn returns_stdout_and_reports_failures() {
        assert_eq!(run(sh("printf ok")).unwrap(), b"ok");
        match run(sh("echo 'scat: bad page' >&2; exit 2")) {
            Err(RunError::Failed(message)) => assert_eq!(message, "bad page"),
            other => panic!("{other:?}"),
        }
        match run(sh("kill -ABRT $$")) {
            Err(RunError::Crashed(signal)) => assert_eq!(signal, libc::SIGABRT),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn applies_resource_limits() {
        let out = run(sh("ulimit -v; ulimit -c")).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert_eq!(text, format!("{}\n0\n", MEMORY_LIMIT / 1024));
    }
}
