//! The plain `cat file` path: copy bytes inside the kernel when the descriptors allow
//! it, as GNU cat does, instead of reading into and writing from a user-space buffer.
//!
//! - a pipe as input: `splice` to standard output;
//! - a regular file to a regular file: `copy_file_range`;
//! - a regular file to anything else (pipe, `/dev/null`, socket): `sendfile`.
//!
//! Whenever the kernel refuses (an `O_APPEND` output, a file system without support,
//! a `/proc` file that reports size 0, an error part-way) the rest of the input is
//! copied with plain reads and writes, which also report errors the way cat does.

use std::io::{self, Write};
use std::os::fd::{AsFd, BorrowedFd};

use rustix::fs::FileType;

use crate::cat;
use crate::input::{Input, InputKind};
use crate::util::TrackedWriter;

/// Bytes asked for per system call; the kernel copies less when it wants to.
const CHUNK: usize = 1 << 30;

/// An output pipe is grown to this size, as GNU cat does: fewer, larger transfers.
/// Within the default unprivileged limit (`/proc/sys/fs/pipe-max-size`, 1 MiB).
const PIPE_SIZE: usize = 512 * 1024;

/// Copies `input` to standard output unchanged. `out` must write to standard output
/// directly (no pager): it is flushed first, then bypassed.
pub fn copy<W: Write>(input: &mut Input, out: &mut TrackedWriter<W>) -> io::Result<()> {
    if let Some(fd) = input.fd
        && input.reader.buffer().is_empty()
    {
        out.flush()?;
        // SAFETY: `fd` belongs to the file or stdin handle inside `input.reader`, which
        // stays open for as long as `input` lives, and nothing has read past it yet.
        let source = unsafe { BorrowedFd::borrow_raw(fd) };
        let stdout = io::stdout();
        if kernel_copy(source, stdout.as_fd(), input.kind) {
            return Ok(());
        }
    }
    cat::copy_raw(&mut input.reader, out)
}

/// Returns true when the whole input was copied. False means "continue with read and
/// write from the current position": nothing is lost, the kernel calls move the file
/// offset only by what they copied.
fn kernel_copy(input: BorrowedFd, output: BorrowedFd, kind: InputKind) -> bool {
    let Ok(out_stat) = rustix::fs::fstat(output) else {
        return false;
    };
    let out_type = FileType::from_raw_mode(out_stat.st_mode);
    if out_type == FileType::Fifo
        && rustix::pipe::fcntl_getpipe_size(output).is_ok_and(|size| size < PIPE_SIZE)
    {
        let _ = rustix::pipe::fcntl_setpipe_size(output, PIPE_SIZE);
    }
    match kind {
        // A size of 0 is how /proc and some /sys files look: read them normally.
        InputKind::Regular { size } if size > 0 => {
            if out_type == FileType::RegularFile {
                let copied =
                    drain(|| rustix::fs::copy_file_range(input, None, output, None, CHUNK));
                match copied {
                    Drained::Done => return true,
                    Drained::Failed { copied: true } => return false,
                    Drained::Failed { copied: false } => {}
                }
            }
            matches!(
                drain(|| rustix::fs::sendfile(output, input, None, CHUNK)),
                Drained::Done
            )
        }
        InputKind::Stream => {
            let is_pipe = rustix::fs::fstat(input)
                .is_ok_and(|st| FileType::from_raw_mode(st.st_mode) == FileType::Fifo);
            is_pipe
                && matches!(
                    drain(|| rustix::pipe::splice(
                        input,
                        None,
                        output,
                        None,
                        CHUNK,
                        rustix::pipe::SpliceFlags::empty(),
                    )),
                    Drained::Done
                )
        }
        _ => false,
    }
}

enum Drained {
    /// The call returned 0 after copying something: end of input.
    Done,
    /// The call failed or returned 0 before copying anything (which may be a file
    /// system that only pretends); `copied` tells whether some bytes went through.
    Failed { copied: bool },
}

fn drain(mut step: impl FnMut() -> rustix::io::Result<usize>) -> Drained {
    let mut copied = false;
    loop {
        match step() {
            Ok(0) if copied => return Drained::Done,
            Ok(0) => return Drained::Failed { copied: false },
            Ok(_) => copied = true,
            Err(rustix::io::Errno::INTR) => {}
            Err(_) => return Drained::Failed { copied },
        }
    }
}
