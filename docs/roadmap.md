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

## Next

| Milestone | Scope |
|---|---|
| M1, text (rest) | inline images in markdown (with M2); paging through `less -R` for long text; hexdump view for binaries |
| M2, images | kitty and sixel output; Unicode block, sextant and ASCII fallbacks; GIF animation; SVG |
| M3, documents and media | PDF pages (hayro, with poppler as fallback); audio tags, cover art and waveform; video metadata and poster frame |
| M4, format pack | JSON/YAML/TOML trees, CSV tables, transparent `.gz/.xz/.zst`, diffs, logs, directories, archives, ELF, SQLite, notebooks, certificates, office documents |

The full list of formats under consideration is in [research.md §8](research.md#8-file-format-index).
