//! Running a diagram engine: source on standard input, SVG back on standard output,
//! with a wall clock timeout and an output cap. The built-in engines (scat itself)
//! also get an address space and CPU time limit and no core dumps; external tools do
//! not, since a JVM or a browser reserves far more address space than it uses.

use std::io::{self, Read, Write};
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Address space limit for the helper.
const MEMORY_LIMIT: u64 = 2 << 30;

/// CPU seconds before the kernel stops the helper (a backstop for the timeout).
const CPU_LIMIT: u64 = 15;

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
            RunError::Spawn(err) => write!(f, "cannot start: {}", crate::util::io_message(err)),
            RunError::TimedOut(limit) => write!(f, "gave up after {} s", limit.as_secs()),
            RunError::Crashed(signal) => write!(f, "crashed (signal {signal})"),
            RunError::Failed(message) if message.is_empty() => f.write_str("failed"),
            RunError::Failed(message) => f.write_str(message),
        }
    }
}

/// Runs `cmd` with `input` on its standard input and returns its standard output (at
/// most `max_output` bytes). `limits` applies the resource limits for scat's helper.
pub fn run(
    mut cmd: Command,
    input: &[u8],
    timeout: Duration,
    max_output: u64,
    limits: bool,
) -> Result<Vec<u8>, RunError> {
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if limits {
        // SAFETY: only setrlimit, which is async-signal-safe, runs between fork and exec.
        unsafe {
            cmd.pre_exec(|| {
                limit(libc::RLIMIT_AS, MEMORY_LIMIT)?;
                limit(libc::RLIMIT_CPU, CPU_LIMIT)?;
                limit(libc::RLIMIT_CORE, 0)?;
                Ok(())
            });
        }
    }
    let mut child = cmd.spawn().map_err(RunError::Spawn)?;
    let stdin = child.stdin.take();
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let input = input.to_vec();
    // A writer thread, so a child that writes before reading everything cannot block.
    let writer = std::thread::spawn(move || {
        if let Some(mut stdin) = stdin {
            let _ = stdin.write_all(&input);
        }
    });
    let out_reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(stdout) = stdout {
            let _ = stdout.take(max_output).read_to_end(&mut buf);
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
                let _ = writer.join();
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
    let _ = writer.join();
    let stdout = out_reader.join().unwrap_or_default();
    let stderr = err_reader.join().unwrap_or_default();
    if let Some(signal) = status.signal() {
        return Err(RunError::Crashed(signal));
    }
    if !status.success() {
        let text = String::from_utf8_lossy(&stderr);
        let line = text.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
        let line = line.trim();
        let line = line
            .strip_prefix("scat: ")
            .or_else(|| line.strip_prefix("Error: "))
            .unwrap_or(line);
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

    fn cmd(program: &str, args: &[&str]) -> Command {
        let mut cmd = Command::new(program);
        cmd.args(args);
        cmd
    }

    #[test]
    fn feeds_stdin_and_caps_stdout() {
        let out = run(cmd("cat", &[]), b"hello", Duration::from_secs(5), 3, false).unwrap();
        assert_eq!(out, b"hel");
    }

    #[test]
    fn reports_failures_and_timeouts() {
        match run(
            cmd("ls", &["/nonexistent-scat-dir"]),
            b"",
            Duration::from_secs(5),
            1024,
            false,
        ) {
            Err(RunError::Failed(message)) => assert!(message.contains("nonexistent")),
            other => panic!("{other:?}"),
        }
        match run(
            cmd("sleep", &["5"]),
            b"",
            Duration::from_millis(100),
            1024,
            true,
        ) {
            Err(RunError::TimedOut(_)) => {}
            other => panic!("{other:?}"),
        }
        assert!(matches!(
            run(
                cmd("/nonexistent-scat-program", &[]),
                b"",
                Duration::from_secs(1),
                1,
                false
            ),
            Err(RunError::Spawn(_))
        ));
    }
}
