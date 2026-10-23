# scat documentation

| Page | What it covers |
|---|---|
| [cat-compatibility.md](cat-compatibility.md) | flags, stdin, exit codes, when scat behaves exactly like cat |
| [using-as-cat.md](using-as-cat.md) | replacing `cat` with scat: the strict `cat` symlink, aliases, what changes on a terminal |
| [rendering.md](rendering.md) | what happens when output is a terminal: text, binary files, escape sequences |
| [paging.md](paging.md) | long output through `less`, file headers for several files |
| [audio.md](audio.md) | audio files: tag table, waveform, `--play` with mpv |
| [markdown.md](markdown.md) | rendered markdown: wrapping, tables, code blocks, links and OSC 8, seeing the source |
| [data-files.md](data-files.md) | CSV/TSV tables, minified JSON pretty-printing, what YAML/TOML do |
| [syntax-highlighting.md](syntax-highlighting.md) | language detection, themes, colour depth |
| [video.md](video.md) | video info view: streams, duration, ffprobe dependency |
| [pdf.md](pdf.md) | PDF documents: info header, pages as images, `--pages`, `--text`, the parser sandbox |
| [binaries.md](binaries.md) | executables, libraries and wasm: ELF, PE, Mach-O and `ar` summaries, security flags, fallbacks |
| [archives.md](archives.md) | zip, tar, tarballs, deb, 7z: summary and file tree, caps, hostile names |
| [office-documents.md](office-documents.md) | Word, PowerPoint, Excel and OpenDocument files shown as markdown: slides, sheets, limits |
| [certificates-and-keys.md](certificates-and-keys.md) | X.509 certificates, CSRs, public keys, SSH key files; private keys with their material hidden |
| [compressed-files.md](compressed-files.md) | transparent `.gz` `.xz` `.zst` `.bz2`: inner type detection, tarballs, size cap |
| [diffs-and-logs.md](diffs-and-logs.md) | coloured unified diffs and log files: levels, timestamps, streaming |
| [images.md](images.md) | images in the terminal: kitty, sixel, block and ASCII output, sizing, animation, SVG |
| [configuration.md](configuration.md) | config file location, `[defaults]`, `[[rule]]`, environment variables |
| [directory-listing.md](directory-listing.md) | `scat` alone and `scat DIR`: grid and long listings, colours, opting out |
| [terminal-detection.md](terminal-detection.md) | how scat learns what the terminal supports, `--diagnose` |
| [install-and-update.md](install-and-update.md) | install script, `scat --update`, pre-releases, uninstalling |
| [releasing.md](releasing.md) | cutting releases and pre-releases (maintainers) |
| [roadmap.md](roadmap.md) | what is done and what comes next |
| [research.md](research.md) | background research: protocols, libraries, prior art, format index |
