# cat compatibility

scat is meant to replace `cat`, so scripts must keep working.

## When scat is exactly cat

scat writes byte-for-byte what GNU cat writes when any of these holds:

- standard output is not a terminal (a pipe, a file, `$(...)`), and `--force-render` is not given;
- `--raw` is given;
- one of cat's visualisation flags is given: `-A`, `-e`, `-E`, `-t`, `-T`, `-v`;
- a config rule sets `renderer = "raw"` for the file;
- the input is a device such as `/dev/urandom` or an interactive terminal.

The test suite (`tests/cat_compat.rs`) compares scat with GNU cat over every flag combination and several tricky files.

## Exceptions when rendering

Two behaviours differ from cat, and only when scat renders (stdout is a terminal or `-f`):

- `scat` with no arguments, when stdin is also a terminal, lists the current directory instead of waiting for input. With stdin redirected (`echo hi | scat`) or stdout piped it reads stdin like cat. Disable with `no_args = "stdin"` under `[defaults]`.
- A directory argument is listed instead of failing. Disable with `directories = "error"` under `[defaults]`.

See [directory-listing.md](directory-listing.md).

## Flags

| Flag | Meaning |
|---|---|
| `-A`, `--show-all` | same as `-vET` |
| `-b`, `--number-nonblank` | number non-empty lines, overrides `-n` |
| `-e` | same as `-vE` |
| `-E`, `--show-ends` | `$` at the end of each line (`^M$` for CRLF, as in coreutils 9) |
| `-n`, `--number`, `--line-numbers` | number all lines |
| `-s`, `--squeeze-blank` | collapse runs of empty lines into one |
| `-t` | same as `-vT` |
| `-T`, `--show-tabs` | show tabs as `^I` |
| `-u` | ignored |
| `-v`, `--show-nonprinting` | `^X` and `M-X` notation |

Line numbering and squeezing continue across files, and a file without a trailing newline runs into the next file's first line, as with cat.

When scat renders (stdout is a terminal), line numbers are on by default and appear in a gutter (`   1 │ `), restarting per file; `-p` hides them. `-b` and `-s` still apply. Piped output only gets numbers with `-n` or `-b`, as with cat.

## Input and errors

- No file, or `-`, reads standard input. `-` can appear several times.
- An unreadable file prints `scat: NAME: reason` to stderr and scat continues with the next file. The exit status is 1 if any file failed, else 0.
- A directory is an error (`Is a directory`), as with cat, when output is piped or `--raw` is given.
- Writing a file onto itself (`scat a >> a`) is refused with `input file is output file`.
- When the reader of a pipe goes away (`scat big.log | head`), scat exits silently through SIGPIPE, like cat.
- Streamed input (`tail -f log | scat`) is written as it arrives.
