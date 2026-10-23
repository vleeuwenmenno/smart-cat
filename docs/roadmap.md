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
- **M4, format pack** (partial)
  - CSV/TSV/PSV tables and minified JSON pretty-printing ([data-files.md](data-files.md))
  - transparent `.gz/.xz/.zst/.bz2` decompression ([compressed-files.md](compressed-files.md))
  - diff and log colouring ([diffs-and-logs.md](diffs-and-logs.md))
- **Output polish**
  - [paging](paging.md) through `less -RFX` (auto: only when output exceeds the screen), file headers for several files
  - [hex dump](rendering.md#hex-dump) for binary files
  - `--completions SHELL` and `--man`, shipped in release archives
- **Directory listing**: grid and `--long` views, `LS_COLORS`, `scat` alone lists the current directory (thumbnails come after image output, M2)

## Next

| Milestone | Scope |
|---|---|
| M2, images (rest) | kitty Unicode placeholders for tmux; octants; iTerm2 inline images |
| M3, documents and media (rest) | PDF pages (hayro, with poppler as fallback) |
| M4, format pack (rest) | archives (tarball and zip listing), ELF, SQLite, notebooks, certificates, office documents |

The full list of formats under consideration is in [research.md §8](research.md#8-file-format-index).
