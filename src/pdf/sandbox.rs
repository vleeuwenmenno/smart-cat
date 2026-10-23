//! Running PDF parsers in a separate process.
//!
//! PDF files are untrusted input and parsers can panic, loop or allocate without bound.
//! Release builds use `panic = "abort"`, so a panic in hayro would kill scat itself. All
//! PDF parsing therefore happens in a child process: scat re-runs its own binary with
//! the hidden first argument [`FLAG`], and the poppler tools run the same way. Every
//! child gets:
//!
//! - an address space limit (`RLIMIT_AS`, [`MEMORY_LIMIT`]) and no core dumps,
//! - a CPU time limit and a wall clock timeout ([`TIMEOUT`]), after which it is killed,
//! - a cap on how much output is read back,
//! - `/dev/null` as standard input, and argv-only arguments.
//!
//! A crash, timeout or error in the child becomes an error message; scat goes on.

use std::ffi::OsStr;
use std::io::{self, Read};
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Hidden first argument that turns scat into the PDF helper process.
pub const FLAG: &str = "--internal-pdf";

/// Wall clock limit for one helper run.
pub const TIMEOUT: Duration = Duration::from_secs(10);

/// Address space limit for helpers.
const MEMORY_LIMIT: u64 = 1 << 30;

/// CPU seconds before the kernel stops a helper (a backstop for the timeout).
const CPU_LIMIT: u64 = 15;

/// Most stdout read back from a helper (a PNG of a page fitted to the terminal is a
/// few MiB at most).
pub const MAX_OUTPUT: u64 = 128 << 20;

#[derive(Debug)]
pub enum RunError {
    /// The program could not be started.
    Spawn(io::Error),
    TimedOut,
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
            RunError::TimedOut => write!(f, "gave up after {} s", TIMEOUT.as_secs()),
            RunError::Crashed(signal) => write!(f, "the parser crashed (signal {signal})"),
            RunError::Failed(message) if message.is_empty() => f.write_str("the parser failed"),
            RunError::Failed(message) => f.write_str(message),
        }
    }
}

/// A command that runs scat's own PDF helper with `args`.
pub fn helper<I, S>(args: I) -> Command
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
    cmd.arg(FLAG).args(args);
    cmd
}

/// Runs `cmd` with the limits described in the module docs and returns its stdout.
pub fn run(cmd: Command) -> Result<Vec<u8>, RunError> {
    run_within(cmd, TIMEOUT)
}

/// [`run`] with its own wall clock limit.
pub fn run_within(mut cmd: Command, timeout: Duration) -> Result<Vec<u8>, RunError> {
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // SAFETY: only setrlimit, which is async-signal-safe, runs between fork and exec.
    unsafe {
        cmd.pre_exec(|| {
            limit(libc::RLIMIT_AS, MEMORY_LIMIT)?;
            limit(libc::RLIMIT_CPU, CPU_LIMIT)?;
            limit(libc::RLIMIT_CORE, 0)?;
            Ok(())
        });
    }
    let mut child = cmd.spawn().map_err(RunError::Spawn)?;
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
                return Err(RunError::TimedOut);
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
    if let Some(signal) = status.signal() {
        return Err(RunError::Crashed(signal));
    }
    if !status.success() {
        let text = String::from_utf8_lossy(&stderr);
        let line = text.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
        let line = line.strip_prefix("scat: ").unwrap_or(line);
        return Err(RunError::Failed(line.chars().take(200).collect()));
    }
    Ok(stdout)
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
