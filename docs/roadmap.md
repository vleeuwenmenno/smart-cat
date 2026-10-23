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
  - [markdown rendering](markdown.md): wrapping, lists, tables, highlighted code blocks, OSC 8 links
- **M2, images**
  - kitty graphics protocol and sixel output, chosen by the terminal query
  - Unicode half-block, sextant and ASCII fallbacks at the detected colour depth
  - GIF, WebP and APNG animation (plays once)
  - SVG via resvg
  - `--protocol`, `--no-animate`, `[image]` config, `renderer = "image"`
- **M3, documents and media** (partial)
  - audio info view: tag table, properties and braille waveform ([audio.md](audio.md))
  - video info: container, duration, bitrate and stream details via ffprobe ([video.md](video.md))
  - `--play` hands audio and video to mpv
- **Directory listing**: grid and `--long` views, `LS_COLORS`, `scat` alone lists the current directory (thumbnails come after image output, M2)

## Next

| Milestone | Scope |
|---|---|
| M1, text (rest) | inline images in markdown; paging through `less -R` for long text; hexdump view for binaries |
| M2, images (rest) | kitty Unicode placeholders for tmux; octants; iTerm2 inline images |
| M3, documents and media (rest) | PDF pages (hayro, with poppler as fallback); audio cover art and video poster frame |
| M4, format pack | JSON/YAML/TOML trees, CSV tables, transparent `.gz/.xz/.zst`, diffs, logs, archives, ELF, SQLite, notebooks, certificates, office documents |

The full list of formats under consideration is in [research.md §8](research.md#8-file-format-index).
