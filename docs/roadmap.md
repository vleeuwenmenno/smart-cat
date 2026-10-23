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
- **M3, documents and media** (partial)
  - audio info view: tag table, properties and braille waveform ([audio.md](audio.md))
  - video info: container, duration, bitrate and stream details via ffprobe ([video.md](video.md))
  - `--play` hands audio and video to mpv
- **Directory listing**: grid and `--long` views, `LS_COLORS`, `scat` alone lists the current directory (thumbnails come after image output, M2)

## Next

| Milestone | Scope |
|---|---|
| M1, text (rest) | markdown rendering; paging through `less -R` for long text; hexdump view for binaries |
| M2, images | kitty and sixel output; Unicode block, sextant and ASCII fallbacks; GIF animation; SVG |
| M3, documents and media (rest) | PDF pages (hayro, with poppler as fallback); audio cover art and video poster frame (need image output from M2) |
| M4, format pack | JSON/YAML/TOML trees, CSV tables, transparent `.gz/.xz/.zst`, diffs, logs, archives, ELF, SQLite, notebooks, certificates, office documents |

The full list of formats under consideration is in [research.md §8](research.md#8-file-format-index).
