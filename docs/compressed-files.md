# Compressed files

When rendering, scat decompresses single-stream `.gz`, `.xz`, `.zst` and `.bz2` files and shows what is inside, as if the file were not compressed. `scat app.log.gz` shows `app.log` with log colours, `notes.md.xz` shows rendered markdown.

## How it works

1. The format is recognised from the magic bytes, not the extension. A gzip file called `blob` is still decompressed.
2. The stream is decoded while it is printed, so a huge file starts showing at once and memory stays small.
3. The first 8 KiB of the decompressed data is sniffed again, with the file name minus the compression suffix (`app.log.gz` becomes `app.log`). The inside gets its normal renderer: syntax highlighting, markdown, [diff and log colours](diffs-and-logs.md), images, or the [hex dump](rendering.md#hex-dump).
4. [Config rules](configuration.md) match the inner name, so `match = { ext = "log" }` also applies to `app.log.gz`. A rule with `renderer = "raw"` that matches the compressed file turns decompression off.

Multi-member gzip, bzip2 and xz files (several streams appended together) are read to the end.

## What is not decompressed

- Piped output. `scat app.log.gz | cat` prints the compressed bytes, exactly like `cat`. `-f` renders into a pipe, and then decompresses.
- Tarballs: `.tar.gz`, `.tgz`, `.tar.xz`, `.txz`, `.tar.zst`, `.tar.bz2`, `.tbz2`, and any stream whose first block is a tar header (`ustar`). They are not a text stream; they get the [archive listing](archives.md) instead.
- Anything with a `raw` renderer rule.

## Limits

Decompressed output stops after `max_decompressed_bytes` (default 1 GiB, set under `[defaults]`) and scat prints a note on stderr: `scat: NAME: decompressed output stopped at 1.0 GiB (max_decompressed_bytes)`. This guards against decompression bombs. xz files that need more than 256 MiB of dictionary memory per block fail with an error instead.

A corrupt or truncated stream prints what could be decoded, then `scat: NAME: ...` and exits with status 1.

## Implementation

All decoders are pure Rust, so the static musl build needs no system library: `flate2` (miniz_oxide), `lzma-rust2` (xz), `ruzstd` (zstd) and `bzip2` (libbz2-rs-sys). zstd files that need a dictionary are not supported.
