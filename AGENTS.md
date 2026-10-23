# AGENTS.md

`scat` (project name smart-cat, crate `smart-cat`, binary `scat`) is a cat replacement for Linux that renders files when writing to a terminal.

## Commands

Use the Makefile targets, not raw cargo commands. They carry `--locked` and the CI flags.

| Task | Command |
|---|---|
| List targets | `make` (the default goal is a coloured help) |
| Install locally | `make install` (native build to `~/.local/bin`, no musl needed) |
| Build (debug) | `make build` |
| Run | `make run ARGS="-n src/main.rs"` |
| Test | `make test` |
| Lint (clippy, warnings are errors) | `make lint` |
| Format | `make fmt` |
| Everything CI checks | `make check` |
| Static release archive | `make dist` (uses `musl-gcc`, falls back to `gcc` for the host CPU; override with `DIST_CC=`) |
| Release | `make bump VERSION=x.y.z` then `git push --follow-tags` |

Keep the help text in the `help` target in sync when adding targets.

Run `make check` before handing work back.

## Layout

| Path | Purpose |
|---|---|
| `src/main.rs` | entry point and dispatch of info commands |
| `src/cli.rs` | clap definition; GNU cat flags plus scat flags |
| `src/cat.rs` | byte-exact GNU cat behaviour (`-n -b -s -E -T -v`) |
| `src/fastcopy.rs` | plain copy inside the kernel (`copy_file_range`, `sendfile`, `splice`) with a read/write fallback |
| `src/render.rs` | per-file dispatch, text rendering, control-character sanitizing |
| `src/highlight.rs` | syntect + two-face (bat's syntaxes and themes), colour reduction |
| `src/audio.rs` + `src/waveform.rs` | audio info view (lofty tags, symphonia peaks, braille/ASCII waveform) |
| `src/listing.rs` + `src/listing/` | directory listing: tiles (`tiles.rs`), icons (`icons.rs`), thumbnails and their cache (`thumbs.rs`), grid and long views |
| `src/play.rs` | `--play`: hands audio and video to mpv |
| `src/pdf/` | PDF view: info header, `--pages` parser, page drawing and text; `helper.rs` runs hayro in a child (`--internal-pdf`), `sandbox.rs` adds rlimits and the timeout |
| `src/office/` | office documents as markdown: `docx.rs`, `pptx.rs`, `odf.rs` (odt/odp) and `sheets.rs` (calamine) build markdown with `md.rs`; caps on parts, rows, slides and output |
| `src/pager.rs` | paging: pager command parsing, decision, `Output` writer that holds back short output |
| `src/header.rs` | file name rule printed before each file when several are rendered |
| `src/hexdump.rs` | hex dump view for binary files |
| `src/binary.rs` | executable/library summary (goblin for ELF/PE/Mach-O/ar, wasmparser) above a short hex dump |
| `src/archive.rs` | archive listing: zip, tar and tarballs, deb, 7z; summary header and file tree |
| `src/cert/` | certificate and key view: PEM scanning, X.509/CSR summaries (x509-parser), key description and OpenSSH fingerprints, `authorized_keys`/`known_hosts`; private key material is never printed |
| `src/generate.rs` | `--completions` and `--man` (clap_complete, clap_mangen) |
| `src/detect.rs` | text/binary and MIME detection from the first 8 KiB |
| `src/input.rs` | opening files and stdin, file kinds |
| `src/config.rs` + `src/config_template.toml` | TOML config, rules, `--init-config` |
| `src/image/` | image view: decoding (image crate, resvg), kitty/sixel/block/ASCII encoders, sizing, animation; `image::show` for other renderers |
| `src/term/` | colour depth, env hints, the batched `/dev/tty` capability query and its cache in `$XDG_RUNTIME_DIR` |
| `src/decompress.rs` | gzip/xz/zstd/bzip2 stream decoders (pure Rust) with a size cap |
| `src/linestyle.rs` | per-line diff and log colouring |
| `src/diagnose.rs` | `--diagnose` report |
| `src/update.rs` | `--update` / `--check-update` from GitHub releases |
| `tests/cat_compat.rs` | byte-for-byte comparison against GNU cat |
| `tests/script_compat.rs` | shell idioms (`sh -c`, `bash -c`) run with GNU cat and scat: output, status, messages |
| `tests/archive.rs` | archive views with zip, tar, tar.gz, 7z and deb fixtures built in the test |
| `tests/cat_mode.rs` | strict mode through a `cat` symlink and the script-safe paging rule, on a pty via `script` |
| `tests/audio.rs` | audio view and `--play` errors, with a WAV generated in the test |
| `tests/binary.rs` | binary summaries: system binaries, a hand-built wasm module, hex fallback |
| `tests/images.rs` | forced image output, captions, piped image bytes |
| `tests/certificates.rs` + `tests/fixtures/test-keys/` | certificate and key view, hidden private keys, piped bytes; the fixtures are test-only keys |
| `tests/pdf.rs` | PDF view with hand-written PDFs: header, pages, text, piped bytes, malformed files |
| `tests/office.rs` | office documents: fixtures zipped in the test, rendered output, row cap, piped bytes, corrupt files |
| `install.sh` | curl-pipe installer (POSIX sh) |
| `docs/` | one page per feature; `docs/research.md` holds the background research |

## Rules that must not break

1. **Piped output is cat.** When stdout is not a terminal (and `--force-render` is not set), output must be byte-identical to GNU cat for the same flags. `tests/cat_compat.rs` enforces this; add fixtures when touching `cat.rs`.
2. **Never pass a file's escape sequences through unchecked** when rendering. Only SGR colour sequences may pass (config `ansi = "keep-sgr"`); everything else is shown as `^X`. Escapes scat emits itself are fine.
3. **A broken config never stops scat from printing.** It becomes a warning and the defaults apply.
4. **External programs run with argv arrays**, never through `sh -c`, and file names are passed as single arguments.
5. **Startup time matters.** Load heavy assets (syntaxes, themes) lazily and only for files that need them. Plain `scat file` must stay close to `cat`.
6. **Terminal queries** only happen on a real terminal, never for `TERM=dumb|linux`, and always restore the terminal mode.

## Conventions

- Rust 2024 edition. Errors to the user follow coreutils style: `scat: NAME: message`.
- New renderers get a `Renderer` variant only when implemented; do not add config keys for features that do not exist yet.
- Each user-facing feature has a page in `docs/`. Update it in the same change as the code. Keep `README.md` short: install, a few examples, link to docs.
- Release builds are static musl binaries for x86_64 and aarch64. Asset names are version-less (`scat-<target>.tar.gz` + `.sha256`) because `install.sh` and `scat --update` depend on them.
