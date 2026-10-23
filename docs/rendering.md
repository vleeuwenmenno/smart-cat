# Rendering

When standard output is a terminal, scat renders files instead of copying bytes. `-f` / `--force-render` renders even into a pipe (useful with `less -R` or for testing). `--raw` turns rendering off.

## What gets rendered

scat reads the first 8 KiB of each file to decide what it is:

| Detected | Default behaviour |
|---|---|
| markdown (`.md`, `.markdown`, `.mdown`, `.mkd`) | [rendered](markdown.md) |
| text with a known language | [syntax highlighted](syntax-highlighting.md) |
| other text | printed, with control characters made safe |
| `.gz` `.xz` `.zst` `.bz2` | [decompressed](compressed-files.md), then the inside is detected again |
| diffs, logs | [coloured](diffs-and-logs.md) |
| audio (`audio/*`, plus `.wv .ape .mpc .spx .aac` by name) | [tag table and waveform](audio.md) |
| zip, tar, tarballs, deb, 7z | [archive listing](archives.md) |
| other binary | [hex dump](#hex-dump) of the first 512 bytes (`--raw` prints the bytes) |

Text means no NUL bytes and at most 5% invalid UTF-8, so Latin-1 files still count as text. The type comes from magic bytes (`infer`), for example `image/png` or `application/x-executable`.

A [config rule](configuration.md) can pick another renderer per file: `code`, `text`, `markdown`, `image`, `audio`, `video`, `table`, `diff`, `log`, `hex` or `raw`.

Long text output is [paged](paging.md), and several files each get a header rule.

## Hex dump

Binary files that no other renderer claims (executables, archives, unknown data) show their first bytes as a dump:

```
00000000  7f 45 4c 46 02 01 01 00  00 00 00 00 00 00 00 00  │·ELF············│
…
… 12.3 MiB more, use --raw for bytes
```

- an offset gutter, 16 bytes per line in two groups of 8, and an ASCII pane (`·` for bytes that are not printable; `.` and `|` without a UTF-8 locale);
- with colours on, bytes are coloured by class: NUL dim, printable ASCII cyan, whitespace green, everything else yellow;
- 512 bytes by default; `[hexdump] max_bytes` in the [config](configuration.md) changes it;
- when the file is longer, a dim last line says how much is left. `scat --raw FILE` or a pipe prints the real bytes;
- `renderer = "hex"` in a rule shows any file this way. Images, audio and video keep their own views.

## Layout

- A line number gutter is shown by default: `   1 │ code`. `-p` / `--plain` hides it for one run; `line_numbers = false` in the [config](configuration.md) hides it by default, per file type if you like. `-n` / `--line-numbers` turns it back on.
- `-b` leaves empty lines unnumbered; `-s` collapses runs of empty lines.
- Tabs are expanded to `tab_width` columns (default 4).
- CRLF line endings are shown as plain line ends.

## Escape sequences in files

Printing an untrusted file straight to a terminal is risky: escape sequences can change the window title, write to the clipboard (OSC 52), move the cursor to hide text, or make some terminals send input back. When rendering, scat:

- shows every control character as `^X` (`^[` for ESC), like `cat -v`;
- keeps SGR colour sequences (`ESC[...m`) so coloured logs still look right, unless `ansi = "escape"` is set, in which case they are shown as `^[[31m`;
- strips SGR sequences when colours are off (`NO_COLOR`, `--color=never`);
- replaces C1 control characters (U+0080 to U+009F) and invalid UTF-8 with `�`;
- shows Unicode bidirectional controls as `<U+202E>` (U+061C, U+200E, U+200F, U+202A to U+202E, U+2066 to U+2069), so they cannot silently reorder a line ("Trojan Source"); ordinary right-to-left text is shown as is;
- resets colours at the end of each file.

File names in scat's own messages on stderr (`scat: NAME: ...`, the binary-file notice) and config paths in warnings get the same treatment, so a file name cannot smuggle an escape sequence to the terminal either.

Piped output is never changed: that is cat's contract.

## Limits

- Files larger than `max_highlight_bytes` (16 MiB by default) are shown without highlighting.
- Lines longer than 1 MiB switch the rest of the file to unhighlighted output.
