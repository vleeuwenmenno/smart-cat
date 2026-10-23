# Archives

In a terminal, `scat foo.zip` prints a summary and the file tree instead of a hex dump. Nothing is extracted, and no file is written.

```
project.tar.gz
  Type        tar archive (gzip)
  Entries     6
  Size        8.7 KiB uncompressed
  Compressed  1.2 KiB (14%)

└── proj/                         2023-11-14
    ├── src/                      2023-11-14
    │   └── main.rs    8.7 KiB  2023-11-14
    ├── latest -> README          2023-11-14
    ├── README             6 B  2023-11-14
    └── run.sh*           10 B  2023-11-14
```

Piped output is the archive bytes, byte for byte, like `cat`.

## Formats

| Format | Detected by | Notes |
|---|---|---|
| zip, `.jar`, `.whl`, `.apk`, `.epub`, `.xpi` | `PK` signature | only the central directory is read; encrypted or exotic compression still lists |
| tar | `ustar` marker, or the `.tar` name | read as a stream; a file on disk is skipped over with seeks |
| tar + gzip, xz, zstd, bzip2 | compression magic, then `.tar.*` name or `ustar` in the first block | decoded as a stream with the pure Rust decoders of [compressed files](compressed-files.md) |
| `.deb` | `ar` signature and `debian-binary` | `control.tar.*` and `data.tar.*` are listed as two sections |
| `.7z` | 7z signature | header only; an encrypted header is an error |

Not supported: `.rar` (no pure Rust reader exists, so it stays a hex dump), `.iso`, and `.a` static libraries.

## Reading the tree

- Directories come first, sorted naturally (`a2` before `a10`), and are coloured with `LS_COLORS` like the [directory listing](directory-listing.md).
- `*` marks an executable, `-> target` a symlink, `=> target` a tar hard link. A symlink whose target the format does not store (zip with compression, 7z) shows `@`.
- The size column is the uncompressed size. The date column appears when the archive stores dates (zip's 1980-01-01 "no date" is hidden).
- Directories that are not stored as entries are implied from file paths.
- Without a UTF-8 locale the tree uses `|--` and `` `-- `` and the footer ends in `...`.

## Large and hostile archives

- Only the first `max_entries` entries are drawn (500 by default); the rest are counted: `… 1,200 more entries`. For `.deb`, the limit applies across both sections.
- The header always counts every entry scanned. A scan stops after 1,000,000 entries; counts then show `+` and a note says so.
- A compressed tarball stops after `max_decompressed_bytes` of [decompressed data](configuration.md#defaults), with a note. A zip bomb costs nothing, since file data is never decompressed.
- Names are untrusted. Control characters and escape sequences are shown as `^[` and `^J`, and one entry is always one row. Names such as `../../etc/passwd` or `/abs/path` are shown as stored (`../`, `/`), so traversal is visible. Nesting deeper than 64 levels is folded into one name.
- Zip and 7z need seeking: a file is reopened by path, and a pipe is buffered in memory up to 256 MiB.
- 7z headers are parsed in a child process (`scat --internal-7z`, the archive on standard input), with the same limits as the [PDF helper](pdf.md): sevenz-rust2 can panic on hostile headers, which would abort a release build. Before parsing, the end header that the 32-byte start header points to must lie inside the file and be at most 64 MiB; otherwise `not a valid 7z archive: the header lies outside the file`.
- A corrupt archive is an error (`scat: NAME: not a valid zip archive: ...`) and scat goes on with the next file. A tar that breaks after some entries shows those entries and a note.

## Options and config

```toml
[archive]
# Entries drawn per archive (per section for .deb); 0 draws all of them.
max_entries = 500
```

`renderer = "archive"` in a [rule](configuration.md) forces this view; `renderer = "hex"` or `--raw` keeps the bytes. Archives page through the [pager](paging.md) like text.
