use std::fs::File;
use std::io::{self, BufReader, Read};
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub enum Source {
    Stdin,
    Path(PathBuf),
}

impl Source {
    pub fn from_args(files: &[PathBuf]) -> Vec<Source> {
        if files.is_empty() {
            return vec![Source::Stdin];
        }
        files
            .iter()
            .map(|f| {
                if f.as_os_str() == "-" {
                    Source::Stdin
                } else {
                    Source::Path(f.clone())
                }
            })
            .collect()
    }

    /// The name used in messages, as cat shows it, but sanitized: a file name can
    /// hold escape sequences that must not reach the terminal through stderr.
    pub fn display_name(&self) -> String {
        match self {
            Source::Stdin => "-".into(),
            Source::Path(p) => crate::render::sanitize_str(&p.to_string_lossy()),
        }
    }

    /// The file name used for rule and syntax matching.
    pub fn file_name(&self) -> Option<String> {
        match self {
            Source::Stdin => None,
            Source::Path(p) => p.file_name().map(|n| n.to_string_lossy().into_owned()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputKind {
    Regular {
        size: u64,
    },
    /// Pipes, sockets and terminals: streamed, sniffed from the first read only.
    Stream,
    /// Character and block devices such as /dev/zero: never sniffed or rendered.
    Device,
}

pub struct Input {
    pub reader: BufReader<Box<dyn Read>>,
    pub kind: InputKind,
    /// Device and inode, to detect "input file is output file".
    pub identity: Option<(u64, u64)>,
    /// The descriptor `reader` reads from, for the kernel copy in `fastcopy`. Only valid
    /// while `reader` is the opened file itself: set to `None` when it is replaced.
    pub fd: Option<RawFd>,
}

pub fn open(source: &Source) -> io::Result<Input> {
    let (reader, meta, fd): (Box<dyn Read>, std::fs::Metadata, RawFd) = match source {
        Source::Stdin => {
            let meta = stdin_metadata()?;
            (Box::new(io::stdin()), meta, io::stdin().as_raw_fd())
        }
        Source::Path(path) => {
            let file = File::open(path)?;
            let meta = file.metadata()?;
            let fd = file.as_raw_fd();
            (Box::new(file), meta, fd)
        }
    };
    let file_type = meta.file_type();
    if file_type.is_dir() {
        return Err(io::Error::from_raw_os_error(libc::EISDIR));
    }
    let kind = if file_type.is_file() {
        InputKind::Regular { size: meta.len() }
    } else if file_type.is_char_device() || file_type.is_block_device() {
        InputKind::Device
    } else {
        InputKind::Stream
    };
    Ok(Input {
        reader: BufReader::with_capacity(64 * 1024, reader),
        kind,
        identity: Some((meta.dev(), meta.ino())),
        fd: Some(fd),
    })
}

fn stdin_metadata() -> io::Result<std::fs::Metadata> {
    // /proc/self/fd/0 follows to whatever stdin is, without taking ownership of fd 0.
    std::fs::metadata("/proc/self/fd/0").or_else(|_| std::fs::metadata("/dev/stdin"))
}

/// Device and inode of stdout when it is a regular file.
pub fn stdout_identity() -> Option<(u64, u64)> {
    let stat = rustix::fs::fstat(io::stdout()).ok()?;
    let is_regular =
        rustix::fs::FileType::from_raw_mode(stat.st_mode) == rustix::fs::FileType::RegularFile;
    is_regular.then_some((stat.st_dev, stat.st_ino))
}

/// GNU cat's "input file is output file" check. Copying a regular file onto itself is
/// only an error when it would never end: when the input's read offset lies before the
/// point where output is written (the end of the file for O_APPEND). A named file is
/// read from offset 0; standard input may already be part-way or at the end.
pub fn copies_onto_itself(source: &Source, input: &Input, out: Option<(u64, u64)>) -> bool {
    let InputKind::Regular { size } = input.kind else {
        return false;
    };
    if out.is_none() || out != input.identity {
        return false;
    }
    let in_pos = match source {
        Source::Stdin => match rustix::fs::tell(io::stdin()) {
            Ok(pos) => pos,
            Err(_) => return false,
        },
        Source::Path(_) => 0,
    };
    let append = rustix::fs::fcntl_getfl(io::stdout())
        .is_ok_and(|flags| flags.contains(rustix::fs::OFlags::APPEND));
    let out_pos = if append {
        size
    } else {
        match rustix::fs::tell(io::stdout()) {
            Ok(pos) => pos,
            Err(_) => return false,
        }
    };
    in_pos < out_pos
}
