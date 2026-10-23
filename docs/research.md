# scat (smart-cat): research

Research date: 2026-10-23. Binary name: `scat`. Crate/project name: `smart-cat`.

Claims marked *(verify)* come from model knowledge or a single secondary source and must be checked before they drive a design decision.

## 1. Goal

A drop-in `cat` for Linux that renders what it prints:

- images (png, jpg, webp, gif with animation, svg, ...)
- markdown
- syntax-highlighted source code
- PDF pages
- audio (mp3, flac, ...): tags, cover art, waveform
- `--line-numbers`
- a user config with per-file-type preferences

It must look good on GPU terminals (kitty, foot, ghostty, wezterm, alacritty) and degrade cleanly to the Linux VT, serial lines and dumb terminals. When stdout is not a TTY it must behave exactly like `cat`.

## 2. Prior art

| Tool | What it does | What scat takes from it | Gap scat fills |
|---|---|---|---|
| **mcat** (Rust, MIT, ~1.4k stars, last commit 2026-10-03) | images, video, md, PDF, office docs, html, mermaid, `ls` with thumbnails | its published crates `rasteroid` and `markdownify`, and its dependency picks (see §4) | env-only terminal detection, no config file, no cat flags, no proper TTY/serial fallback, no audio |
| **bat** | syntax highlighting, paging, cat-compat when piped | `--style`, `--plain`, `--paging=auto`, `BAT_*` env pattern, `--sanitize` (added 2026), lessons from CVE-2021-36753 (pager resolved from CWD) | everything non-text |
| **chafa** (C, LGPL) | best symbol fallback (blocks, sextants, braille, ASCII) with error-minimising symbol choice | the symbol-selection approach | it is an image tool only |
| **timg** | images and video via kitty, sixel, iTerm2 and half-blocks | redraw-loop animation model | — |
| **glow / mdcat** | markdown rendering; mdcat shows inline images | mdcat never mixes images with a pager | — |
| **hexyl** | coloured hex dump | binary fallback view | — |
| **yazi / ranger scope.sh / pistol / ctpv / lesspipe** | mime → previewer rule tables | first-match `[[rule]]` table with mime globs | not cat-like |
| **tdf, tpdf, termpdf.py, fancy-cat** | interactive PDF viewers (pdfium or MuPDF + kitty protocol) | render page → scale to cell pixel size → send | interactive, not cat |

No existing tool combines cat compatibility, per-type config, active capability detection and a real text-terminal fallback. mcat is the nearest competitor and a source of reusable code.

### Reusable mcat crates (both MIT, on crates.io)

- **`rasteroid` 0.4.0**: kitty, sixel and iTerm2 encoders, a half-block ASCII encoder, frame encoding for GIF/video, and kitty shared-memory transfer. Weakness: `is_kitty_capable` / `is_sixel_capable` only read env vars (`TERM`, `TERM_PROGRAM`, `LC_TERMINAL`, tmux original term). It never queries the terminal, so it is wrong over SSH and in many multiplexer setups. scat should put its own detection layer in front and call the encoders directly.
- **`markdownify` 0.3.8**: a Rust "markitdown". Converts docx, pptx, xlsx/xls/ods (via `calamine`), odt/odp, csv, zip/tar listings to markdown. scat can then render that markdown with its own renderer.

## 3. Terminal output

### 3.1 Graphics protocols

| Protocol | Colour | Animation | tmux | Notes |
|---|---|---|---|---|
| **kitty graphics** | RGB/RGBA/PNG, alpha | native: upload frames once, the terminal loops them | needs `allow-passthrough on`; **Unicode placeholders** (U+10EEEE, kitty ≥0.28) are the reliable tmux mode | transmission by base64 chunks (works over SSH), file, temp file or shared memory (local only) |
| **Sixel** | palette, usually 256 registers, no alpha | client redraw loop only | tmux ≥3.4 built with sixel *(verify)* | height a multiple of 6; slow and bulky; sizing quirks per terminal |
| **iTerm2 OSC 1337** | whole PNG/JPEG/GIF file, terminal decodes | iTerm2 only | passthrough | supported by wezterm, mintty, rio, partly konsole |

Spec: <https://sw.kovidgoyal.net/kitty/graphics-protocol/>. Sixel list: <https://www.arewesixelyet.com/>.

### 3.2 Support matrix (2026)

| Terminal | kitty | sixel | iTerm2 | scat default |
|---|---|---|---|---|
| kitty | yes | no | no | kitty |
| ghostty | yes | no | no | kitty |
| wezterm | partial | yes | yes | kitty |
| konsole | partial | yes | partial | kitty |
| foot | no | yes | no | sixel |
| xterm | no | yes (`-ti 340`) | no | sixel |
| Windows Terminal, contour, mlterm | — | yes | — | sixel |
| **alacritty** | no | no (sixel PR open for years, unmerged) | no | Unicode blocks, truecolor |
| GNOME Terminal / VTE | no | no (removed from stable in 0.65.91) | no | Unicode blocks |
| Linux VT (`TERM=linux`) | no | no | no | half-blocks, 16 colours (optional `/dev/fb0`) |
| serial / `TERM=dumb` | no | no | no | ASCII ramp or metadata only |

The local machine has kitty, foot, ghostty and alacritty configs, which covers the kitty, sixel and text-only tiers.

### 3.3 Text fallbacks (best first)

1. Octants, 2×4 per cell (Unicode 16, U+1CD00). foot ≥1.20 renders them; font coverage elsewhere is poor. Opt-in only.
2. Sextants, 2×3 (U+1FB00). chafa uses them since 1.8.
3. Quadrants, 2×2.
4. Half-blocks `▀▄`, 1×2 with fg+bg colour. Works almost everywhere, including the VT.
5. Braille, 2×4 dots, one colour per cell. Good for waveforms, maps and line art.
6. ASCII ramp.

Colour depth: 24-bit if `COLORTERM=truecolor|24bit` or the terminal confirms it, else 256, else 16. Pick the symbol plus fg/bg pair with the lowest per-cell error, as chafa does.

### 3.4 Capability detection

1. **stdout not a TTY**: no queries, no escapes. Output bytes as `cat` would, unless `--force-render` is given.
2. **Env hints**: `TERM`, `TERM_PROGRAM`, `KITTY_WINDOW_ID`, `GHOSTTY_RESOURCES_DIR`, `WEZTERM_EXECUTABLE`, `KONSOLE_VERSION`, `VTE_VERSION`, `COLORTERM`, `TMUX`, `ZELLIJ`, `SSH_TTY`. These are hints only, because they lie over SSH and inside multiplexers.
3. **One batched query on `/dev/tty`**, with DA1 as the sentinel. Send in raw mode:
   - kitty query `ESC_Gi=31,s=1,v=1,a=q,t=d,f=24;AAAA ESC\`
   - XTVERSION `CSI > q`
   - cell size `CSI 16 t` and window size `CSI 14 t`
   - background colour `OSC 11;? ST` (for picking a light or dark theme)
   - DA1 `CSI c` last

   Every terminal answers DA1, so anything not answered before the DA1 reply is unsupported, with no per-query timeout. DA1 attribute `4` means sixel. Use a 200 ms hard cap in case DA1 itself never comes back.
4. **Skip queries** for `TERM=linux`, `dumb` or unset.
5. **Cell pixel size**: `TIOCGWINSZ` `ws_xpixel`/`ws_ypixel` first (often 0), then `CSI 16 t`, then assume 8×16.
6. **Restore termios** on every exit path, including SIGINT and panic.
7. `scat --diagnose` prints each result and whether it came from env or was confirmed by a query.

### 3.5 Animation and video

- GIF/APNG/animated WebP on kitty: upload frames with `a=f`, play with `a=a`. Near-zero cost over SSH.
- Elsewhere: redraw loop with synchronized output (`CSI ? 2026 h/l`), cursor hidden, frames pre-encoded, SIGINT restores state. Cap the loop count (cat must terminate): default one loop, `--loop N`.
- Video playback: hand off to `mpv --vo=kitty|sixel|tct` behind `--play`. Default is metadata plus a poster frame.

## 4. Stack: Rust

Reasons: about 1 ms startup with no runtime (matters for a `cat` replacement); memory safety for parsing untrusted files; best ecosystem for highlighting and image decoding; Rust 1.98 is installed locally; mcat's crates are reusable. Go (chroma, glamour) is viable but slower to start and weaker on images. Zig has no ecosystem for this.

| Concern | Pick | Notes |
|---|---|---|
| CLI | `clap` (derive, env) | |
| Config | `serde` + `toml` | |
| File type | `infer` (magic) → extension → `content_inspector` (text/binary) | no libmagic |
| Highlighting | `syntect` (`fancy-regex`, no onig) + `two-face` | bat's syntax set; load lazily |
| Markdown | `comrak` (GFM, has syntect integration) or `pulldown-cmark` | mcat uses comrak |
| Images | `image` with `Limits`, `resvg` (svg), `jxl-oxide` (jxl); avif behind a feature | HEIC through `ffmpeg` or `libheif` feature only |
| Graphics output | `rasteroid` encoders + own detection layer + own sextant/braille encoder | |
| PDF | `hayro` 0.7 (pure Rust, MIT) in a child process; fallback `pdftoppm`; text via `pdftotext -layout` | avoid MuPDF bindings (AGPL) |
| Office / sheets / archives | `markdownify` | |
| Audio | `lofty` (tags, cover art, properties); `symphonia` + braille for waveform; `rustfft` for an optional spectrogram | no `rodio`/`cpal`; playback via `mpv --no-video` |
| Video | `ffmpeg-sidecar` / `ffprobe`; fallback `mp4parse`, `matroska` | |
| Terminal | `rustix` / `libc` for termios and ioctl; `crossterm` only if needed | |

Release profile: `lto = "fat"`, `codegen-units = 1`, `strip = true`, `panic = "abort"`. Heavy renderers sit behind cargo features. Measure startup with `hyperfine` against `cat` and `bat`.

## 5. CLI design

### 5.1 cat compatibility (non-negotiable)

- Multiple files, `-` for stdin, stdin when no args.
- GNU flags `-n -b -s -A -v -e -E -t -T -u`, plus `--line-numbers` as the long form of `-n`.
- On error, report `scat: PATH: reason` and continue; exit 1 if any file failed.
- Handle `EPIPE` quietly (`scat x | head`).
- Detect "input file is output file".

### 5.2 Modes

| Situation | Behaviour |
|---|---|
| stdout not a TTY | identical bytes to `cat`; only explicit transforms like `-n` apply |
| `--force-render` / `-f` | render even when piped (colour per `--color`) |
| `--plain` / `-p` | no decoration, but syntax highlighting is still allowed |
| `--raw` | exactly `cat` |
| `--paging=auto` | page text through `less -RFX` only when output exceeds one screen **and** contains no graphics |

### 5.3 Flags

`--color auto|always|never`, `--theme`, `--language/-l`, `--protocol auto|kitty|sixel|iterm|blocks|sextant|braille|ascii|none`, `--width`/`--height`, `--pages 1-3,7`, `--loop N`, `--play`, `--wrap`, `--tabs`, `--config PATH`, `--no-config`, `--diagnose`, `--print-config`, `--list-renderers`.

Also honour `NO_COLOR`, `CLICOLOR`, `CLICOLOR_FORCE`, `TERM=dumb`.

### 5.4 Default per type

| Type | TTY with graphics | TTY text-only | Piped |
|---|---|---|---|
| source code | highlighted, optional numbers | highlighted (16/256 colour) | raw bytes |
| markdown | styled, inline images | styled, images as `[img: alt]` | raw bytes |
| image | image fitted to width; EXIF line optional | sextant/half-block art | raw bytes |
| gif | animation (one loop) | first frame | raw bytes |
| PDF | page 1 + "N more pages" header; `--pages` | `pdftotext -layout` text | raw bytes (`-f` gives text) |
| audio | cover + tag table + waveform | tag table + braille waveform | raw bytes |
| video | poster frame or contact sheet + metadata | metadata | raw bytes |
| unknown binary | hexyl-style dump (capped) | same | raw bytes |

## 6. Config

Location: `$SCAT_CONFIG`, else `$XDG_CONFIG_HOME/scat/config.toml`, else `~/.config/scat/config.toml`.

Precedence: CLI flags > `SCAT_*` env > first matching rule > `[defaults]` > built-ins.

```toml
[defaults]
line_numbers = false
theme = "auto"            # picked from OSC 11 background
paging = "never"
protocol = "auto"
max_file_bytes = "100MiB"
max_image_pixels = 40_000_000

[[rule]]
match = { ext = ["md", "markdown"] }
renderer = "markdown"

[[rule]]
match = { mime = "image/*" }
renderer = "image"
max_size = { cols = 120, rows = 40 }

[[rule]]
match = { ext = ["rs", "py"] }
line_numbers = true

[[rule]]
match = { ext = ["pdf"] }
pages = "1"

[[rule]]                                   # external handler: argv only, never sh -c
match = { ext = ["parquet"] }
command = ["duckdb", "-c", "SELECT * FROM '{file}' LIMIT 20"]
```

Rules are matched against the detected mime first and the extension as a hint, and the first match wins (as in yazi). `scat --print-config` shows the merged result. `scat --diagnose FILE` shows which rule matched.

## 7. Safety

- **Escape injection**: in rendered text, show C0/C1 control characters as `^[` (as `cat -v` does) and emit only scat's own escapes. Sanitize filenames in headers too. Raw passthrough happens only when piped (cat semantics) or with `--raw`.
- **Decompression bombs**: read dimensions from the header, then decode with `image::Limits`. Cap frames, pages, archive entries and decompressed bytes.
- **Untrusted parsers**: PDF, fonts and media go through a child process with an rlimit, a timeout and (later) landlock/seccomp. Never run PDF JavaScript or actions.
- **External handlers**: argv arrays, filename passed as one argument after `--`, absolute or trusted-PATH resolution, never from CWD.
- **Special files**: `fstat` after open. Devices, FIFOs and sockets are streamed raw and never sniffed or rendered.
- **Huge files**: stream text; sniff only the first 64 KiB; disable highlighting above a size or line-length cap.
- **Secrets**: private keys and `.env` values are masked when rendered (raw when piped, like cat).

## 8. File format index

**Core (v1):** code, markdown, png/jpg/webp/gif/bmp/ico/qoi/tiff, svg, PDF, mp3/flac/ogg/opus/m4a/wav, plain text, hexdump fallback.

### Top 15 after core

| # | Format | Rendering | Crates / tools | Effort |
|---|---|---|---|---|
| 1 | JSON / JSONL / YAML / TOML | pretty-print + tree, depth limit | `serde_json`, `saphyr`, `toml` | S |
| 2 | CSV / TSV | boxed table, delimiter sniffing, numeric alignment | `csv`, `comfy-table` | S |
| 3 | `.gz .xz .zst .bz2` single stream | transparent decompress, then re-detect | `flate2`, `xz2`/`lzma-rust2`, `zstd`, `bzip2` | S |
| 4 | diff / patch | coloured hunks, word-level changes | `similar` | S |
| 5 | log files | level colouring, timestamp gutter, repeat folding | regex | S |
| 6 | directory | tree listing, gitignore aware (mcat-style thumbnails optional) | `ignore` | S |
| 7 | zip / tar / jar / whl / deb | file tree with sizes | `markdownify`, `zip`, `tar` | S |
| 8 | ELF / PE / wasm | arch, sections, needed libs, symbols summary | `goblin`, `wasmparser` | M |
| 9 | HTML | readable text, links as footnotes | `html2text` / `scraper` | M |
| 10 | SQLite | tables, columns, row counts, sample rows | `rusqlite` (bundled) | M |
| 11 | Jupyter `.ipynb` | highlighted cells + text and image outputs | `serde_json` | M |
| 12 | image EXIF | camera, lens, exposure, GPS line | `kamadak-exif` | S |
| 13 | PEM / X.509 / SSH keys | subject, SANs, validity, fingerprint; never key material | `x509-parser`, `ssh-key` | M |
| 14 | docx / xlsx / pptx / odt / ods | via markdown | `markdownify` (`calamine`) | S |
| 15 | video | ffprobe metadata + poster frame | `ffmpeg-sidecar` | M |

### Later / long tail

- **Data**: parquet/arrow (`parquet`), avro, xml/plist (`quick-xml`), npy/npz (`npyz`), hdf5, netcdf.
- **Docs**: epub, man/roff, org (`orgize`), rst, asciidoc, rtf, latex/typst (typst renderer as in mcat).
- **Images**: avif, heic (ffmpeg), jxl (`jxl-oxide`), psd, exr/hdr (tonemap), camera RAW (embedded JPEG), QR decode (`rqrr`).
- **Media**: srt/vtt/ass subtitles, midi (`midly`, piano roll), tracker modules, m3u/pls playlists.
- **Diagrams**: graphviz `.dot` (`dot` → `resvg`), mermaid (`mermaid-rs-renderer`, used by mcat), plantuml (external).
- **Fonts**: ttf/otf/woff2 name table + sample text rendered as an image (`ttf-parser`, `fontdue`).
- **Geo/3D**: geojson/gpx/kml braille map, stl/obj/gltf wireframe.
- **Misc**: ics, vcf, eml/mbox (`mail-parser`), torrent, `.desktop`, har, git objects (`gix`), `.env` with masking.

## 9. Proposed milestones

1. **M0, skeleton (~1 day)**: cargo workspace, `scat` binary, cat-compatible core (flags, stdin, EPIPE, exit codes), TTY detection, config loading, `--diagnose`.
2. **M1, text (~2 days)**: syntax highlighting, `--line-numbers`, markdown, sanitizing, paging.
3. **M2, images (~3 days)**: capability query layer, kitty/sixel via `rasteroid`, sextant/half-block/ASCII fallback, GIF animation, svg.
4. **M3, documents and media (~3 days)**: PDF (hayro + poppler fallback), audio tags/cover/waveform, video poster frame.
5. **M4, format pack**: the top-15 list above, one renderer per PR.

## 10. Open items to verify

1. hayro's robustness on a real PDF corpus (encrypted PDFs and some blend modes are unsupported).
2. Whether `rasteroid`'s API can be used without its env detection, or needs a fork or wrapper.
3. libchafa linking: only matters if we use it instead of our own symbol encoder (LGPL).
4. tmux sixel version, zellij kitty passthrough, octant support in kitty/ghostty.
5. Startup time and binary size budget (target: under 5 ms for plain text, under 10 MB).

## Sources

- mcat: <https://github.com/Skardyy/mcat> (cloned and inspected at commit `ef95b85`, 2026-10-03)
- kitty graphics protocol: <https://sw.kovidgoyal.net/kitty/graphics-protocol/>
- Are We Sixel Yet: <https://www.arewesixelyet.com/>
- ratatui-image: <https://github.com/benjajaja/ratatui-image>
- mpv VO docs: <https://raw.githubusercontent.com/mpv-player/mpv/master/DOCS/man/vo.rst>
- VTE sixel: <https://gitlab.gnome.org/GNOME/vte/-/issues/165>, <https://bugs.archlinux.org/task/79469>
- foot 1.20 (octants): <https://salsa.debian.org/birger/foot/-/tags/1.20.0>
- chafa 1.8 (sextants): <https://hpjansson.org/blag/2021/09/16/chafa-1-8-terminal-graphics-with-a-side-of-everything/>
- terminal comparison: <https://terminaltrove.com/compare/terminals/>
- hayro: <https://docs.rs/hayro>
- pdfium-render: <https://docs.rs/pdfium-render>
- tpdf: <https://aur.archlinux.org/packages/tpdf>, termpdf.py: <https://github.com/sschober/termpdf.py>
- image crate: <https://github.com/image-rs/image>
- bat escape-sequence work: <https://github.com/sharkdp/bat/issues?q=escape+sequence>
- bat CVE-2021-36753: <https://advisories.gitlab.com/pkg/cargo/bat/CVE-2021-36753/>
