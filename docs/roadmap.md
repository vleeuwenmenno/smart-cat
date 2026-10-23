# Roadmap

Background and library choices are in [research.md](research.md).

## Done

- **M0, core**
  - GNU cat compatible output and flags, byte-identical when piped
  - config file with rules
  - terminal capability query and `--diagnose`
  - `--update`, install script, CI and releases
- **M1, text** (partial)
  - syntax highlighting with automatic light/dark theme
  - line number gutter
  - control-character sanitizing
  - [markdown rendering](markdown.md): wrapping, lists, tables, highlighted code blocks, OSC 8 links, local images drawn inline
- **M2, images**
  - kitty graphics protocol and sixel output, chosen by the terminal query
  - Unicode half-block, sextant and ASCII fallbacks at the detected colour depth
  - GIF, WebP and APNG animation (plays once)
  - SVG via resvg
  - `--protocol`, `--no-animate`, `[image]` config, `renderer = "image"`
- **M3, documents and media** (partial)
  - audio info view: tag table, properties, braille waveform and cover art ([audio.md](audio.md))
  - video info: poster frame via ffmpeg; container, duration, bitrate and stream details via ffprobe ([video.md](video.md))
  - `--play` hands audio and video to mpv
  - [PDF](pdf.md): info header, pages drawn with hayro in a sandboxed child process (poppler fallback), `--pages`, text via `pdftotext`
- **M4, format pack** (partial)
  - [office documents](office-documents.md): docx, odt, pptx, odp, xlsx, xls, ods as markdown, with slide and sheet sections
  - [SQLite databases](sqlite.md): header, tables with row counts, schemas, indexes and sample rows, opened read-only and immutable
  - [Jupyter notebooks](notebooks.md): markdown and code cells, text, error and image outputs
  - [subtitles](subtitles.md): srt, vtt, ass and ssa with an overview (span, language, overlap and reading-speed warnings) and the cue list
  - [diagrams](diagrams.md): mermaid (built in, `mmdc` fallback), graphviz (`dot`, built-in `layout-rs` fallback) and PlantUML (`plantuml`) drawn as images, also fenced in markdown
  - CSV/TSV/PSV tables and minified JSON pretty-printing ([data-files.md](data-files.md))
  - transparent `.gz/.xz/.zst/.bz2` decompression ([compressed-files.md](compressed-files.md))
  - diff and log colouring ([diffs-and-logs.md](diffs-and-logs.md))
  - [executable and library summaries](binaries.md): ELF, PE, Mach-O, WebAssembly and `ar` archives, above a short hex dump
  - [archive listing](archives.md): zip, tar, tarballs, deb and 7z as a file tree with sizes and dates
  - [certificates and keys](certificates-and-keys.md): X.509 and CSR summaries with expiry colours and chains, public keys, `authorized_keys`/`known_hosts`, private keys with the material hidden
- **Output polish**
  - [paging](paging.md) through `less -RFX` (auto: only when output exceeds the screen), file headers for several files
  - [hex dump](rendering.md#hex-dump) for binary files
  - `--completions SHELL` and `--man`, shipped in release archives
- **Directory listing**: tiles with thumbnails (images, video frames, PDF pages, cover art) and icons, grid and `--long` views, `LS_COLORS`, a thumbnail cache that also reads freedesktop thumbnails, `scat` alone lists the current directory ([directory-listing.md](directory-listing.md))

## Next

| Milestone | Scope |
|---|---|
| M2, images (rest) | kitty Unicode placeholders for tmux; octants; iTerm2 inline images |
| M3, documents and media (rest) | landlock/seccomp for the PDF helper |
| M4, format pack (rest) | PGP keys |

The full list of formats under consideration is in [research.md §8](research.md#8-file-format-index).
