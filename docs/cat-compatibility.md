# cat compatibility

scat is meant to replace `cat`, so scripts must keep working.

## When scat is exactly cat

scat writes byte-for-byte what GNU cat writes when any of these holds:

- standard output is not a terminal (a pipe, a file, `$(...)`), and `--force-render` is not given;
- `--raw` is given;
- one of cat's visualisation flags is given: `-A`, `-e`, `-E`, `-t`, `-T`, `-v`;
- a config rule sets `renderer = "raw"` for the file;
- the input is a device such as `/dev/urandom` or an interactive terminal;
- scat was started under the name `cat` (a `cat -> scat` symlink), unless `[cat_mode] render = true` is set or `-f` is given. Then scat is cat on a terminal too: no rendering, pager, directory listing, listing for no arguments or terminal query. See [using-as-cat.md](using-as-cat.md).

The test suites (`tests/cat_compat.rs`, `tests/script_compat.rs`) compare scat with GNU cat over every flag combination, several tricky files and common shell idioms. `tests/script_compat.rs` runs every idiom twice: as `scat`, and through a `cat` symlink, where even the error messages must be identical to GNU cat's. `tests/cat_mode.rs` checks the symlink on a pseudo-terminal.

## Known differences in piped output

These only show up in unusual setups:

- Standard input or output closed (`scat <&-`, `scat >&-`): the Rust runtime opens `/dev/null` in their place before scat starts, so scat reads nothing or writes nowhere and exits 0. cat reports `Bad file descriptor` and exits 1.
- `POSIXLY_CORRECT`: cat then treats everything after the first file name as a file (`cat f -n` looks for a file `-n`); scat always accepts options anywhere.
- Message wording differs in details (and messages say `scat:` unless scat runs as `cat`): cat quotes odd names (`cat: 'a b': No such file`), scat prints them as given with control characters made visible.

## Exceptions when rendering

Two behaviours differ from cat, and only when scat renders (stdout is a terminal or `-f`):

- `scat` with no arguments, when stdin is also a terminal, lists the current directory instead of waiting for input. With stdin redirected (`echo hi | scat`) or stdout piped it reads stdin like cat. Disable with `no_args = "stdin"` under `[defaults]`.
- A directory argument is listed instead of failing. Disable with `directories = "error"` under `[defaults]`.

See [directory-listing.md](directory-listing.md). Everything else that changes when scat renders, and how to turn each part off, is in [using-as-cat.md](using-as-cat.md).

## Flags

| Flag | Meaning |
|---|---|
| `-A`, `--show-all` | same as `-vET` |
| `-b`, `--number-nonblank` | number non-empty lines, overrides `-n` |
| `-e` | same as `-vE` |
| `-E`, `--show-ends` | `$` at the end of each line (`^M$` for CRLF, as in coreutils 9, also when the CR and LF fall in different reads or files) |
| `-n`, `--number`, `--line-numbers` | number all lines |
| `-s`, `--squeeze-blank` | collapse runs of empty lines into one |
| `-t` | same as `-vT` |
| `-T`, `--show-tabs` | show tabs as `^I` |
| `-u` | ignored |
| `-v`, `--show-nonprinting` | `^X` and `M-X` notation |

Repeating a flag (`-nn`, `-E -E`) is accepted, as with cat.

Line numbering and squeezing continue across files, and a file without a trailing newline runs into the next file's first line, as with cat.

When scat renders (stdout is a terminal), line numbers are on by default and appear in a gutter (`   1 │ `), restarting per file; `-p` hides them. `-b` and `-s` still apply. Piped output only gets numbers with `-n` or `-b`, as with cat.

## Input and errors

- No file, or `-`, reads standard input. `-` can appear several times.
- An unreadable file prints `scat: NAME: reason` to stderr and scat continues with the next file. The exit status is 1 if any file failed, else 0. Control characters in NAME are shown as `^X` (stderr messages are not part of cat's byte-for-byte contract).
- A directory is an error (`Is a directory`), as with cat, when output is piped or `--raw` is given.
- Writing a file onto itself (`scat a >> a`) is refused with `input file is output file`. As in GNU cat, this only happens when the copy would never end: the input is read from before the point where output is written (the end of the file for `>>`). `scat < a >> a` with standard input already at the end, or `scat a 1<> a`, is not an error.
- A bad option prints `scat: invalid option -- 'z'` (or `unrecognized option '--x'`) and `Try 'scat --help' for more information.`, and exits with status 1, as cat does. Long options may be abbreviated when the prefix is unique (`--show-a`, `--squeeze`).
- An empty file name (`scat ''`) is a missing file, as with cat.
- Messages start with `scat:`. Started under the name `cat` (a symlink), they start with `cat:` and the hint says `Try 'cat --help'`, so they match GNU cat's.
- A plain copy (no flags) moves the bytes inside the kernel where it can (`copy_file_range`, `sendfile`, `splice`), as GNU cat does, so `scat big > copy` or `scat big | tool` is as fast as cat.
- When the reader of a pipe goes away (`scat big.log | head`), scat exits silently through SIGPIPE, like cat.
- Streamed input (`tail -f log | scat`) is written as it arrives, also with `-n`, `-s`, `-E` and the other cat flags: a partial line shows up before its newline, as with cat.
