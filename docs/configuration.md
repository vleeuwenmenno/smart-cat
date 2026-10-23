# Configuration

## Location

scat reads the first of:

1. `--config PATH` or the `SCAT_CONFIG` environment variable;
2. `$XDG_CONFIG_HOME/scat/config.toml`;
3. `~/.config/scat/config.toml`.

`scat --init-config` writes a commented template to that location. `--no-config` ignores the file. `scat --print-config` prints the effective configuration.

A config with mistakes (bad TOML, unknown keys, wrong values) produces a warning on stderr and scat continues with the defaults, so a typo never breaks `cat` in a script.

## Precedence

Command line flags, then `SCAT_*` environment variables, then matching `[[rule]]` entries, then `[defaults]`.

## `[defaults]`

| Key | Default | Meaning |
|---|---|---|
| `color` | `"auto"` | `"auto"`, `"always"` or `"never"` |
| `theme` | unset | fixed theme for every file; unset means choose by background |
| `theme_dark` | `"Monokai Extended"` | theme for dark backgrounds |
| `theme_light` | `"Monokai Extended Light"` | theme for light backgrounds |
| `line_numbers` | `true` | show the line number gutter when rendering (`-p` hides it for one run) |
| `tab_width` | `4` | tab expansion when rendering; `0` keeps tabs |
| `ansi` | `"keep-sgr"` | escape sequences in files: `"keep-sgr"` keeps colours, `"escape"` shows them as `^[` |
| `max_highlight_bytes` | `16777216` | larger files are not highlighted |
| `max_decompressed_bytes` | `1073741824` | [decompression](compressed-files.md) stops after this many bytes |
| `no_args` | `"list"` | `scat` with no arguments, run from a terminal: `"list"` lists the current directory, `"stdin"` reads stdin like cat |
| `directories` | `"list"` | directory arguments when rendering: `"list"` or `"error"` (cat's `Is a directory`) |
| `paging` | `"auto"` | `"auto"` pages long text output, `"always"`, `"never"`; see [paging.md](paging.md) (`--paging` for one run) |
| `headers` | `true` | a rule with the file name before each file when several are rendered (`-p` hides it for one run) |

## `[list]`

| Key | Default | Meaning |
|---|---|---|
| `style` | `"auto"` | `"auto"` (tiles when images can be drawn, else grid), `"tiles"`, `"grid"` or `"long"` (`--style`, `--long` for one run) |
| `show_hidden` | `false` | show dot entries (`--all` for one run) |
| `tile_width` | `16` | width of a tile's thumbnail box in cells (`--tile-size WxH` for one run) |
| `tile_height` | `8` | height of a tile's thumbnail box in cells; the name goes below |
| `thumbnails` | `true` | thumbnails of images, videos, PDFs and audio covers in tiles (`--no-thumbnails` for one run) |
| `icons` | `"unicode"` | icons for entries without a thumbnail: `"unicode"`, `"nerd"` (needs a Nerd Font) or `"none"` |
| `max_thumbnails` | `200` | most thumbnails per listing; the rest get icons |
| `cache` | `true` | read freedesktop thumbnails and keep scat's own in `$XDG_CACHE_HOME/scat/thumbnails` |

See [directory-listing.md](directory-listing.md).

## `[image]`

| Key | Default | Meaning |
|---|---|---|
| `protocol` | `"auto"` | `"auto"`, `"kitty"`, `"sixel"`, `"blocks"`, `"sextants"` or `"ascii"`; see [images.md](images.md) |
| `max_height` | unset | maximum image height in rows; unset means the terminal height minus 4 (at least 10) |
| `animate` | `true` | play animated GIF/WebP/PNG once; `false` shows the first frame |

## `[table]`

| Key | Default | Meaning |
|---|---|---|
| `max_rows` | `200` | data rows shown for CSV/TSV/PSV files before the "more rows" footer; `0` shows all; see [data-files.md](data-files.md) |

## `[hexdump]`

| Key | Default | Meaning |
|---|---|---|
| `max_bytes` | `512` | leading bytes of a binary file shown as a [hex dump](rendering.md#hex-dump) (at least 16) |

## `[binary]`

| Key | Default | Meaning |
|---|---|---|
| `hex_bytes` | `256` | leading bytes shown as a hex dump under the summary of an [executable or library](binaries.md) (at least 16) |

## `[pdf]`

| Key | Default | Meaning |
|---|---|---|
| `pages` | `"1"` | pages shown when `--pages` is not given: `"1"`, `"1-3,7"`, `"4-"` or `"all"`; an invalid value makes the whole config fall back to defaults, with a warning |
| `max_pages` | `20` | most pages drawn as images per file; text output is not limited |

See [pdf.md](pdf.md).

## `[archive]`

| Key | Default | Meaning |
|---|---|---|
| `max_entries` | `500` | entries drawn per [archive listing](archives.md); the rest are counted in a footer. `0` draws all |

## `[office]`

| Key | Default | Meaning |
|---|---|---|
| `max_rows` | `200` | data rows shown per spreadsheet sheet before the "more rows" note; `0` shows all; see [office-documents.md](office-documents.md) |

## `[cat_mode]`

Applies when scat is started under the name `cat`, through a `cat -> scat` symlink (see [using-as-cat.md](using-as-cat.md)).

| Key | Default | Meaning |
|---|---|---|
| `render` | `false` | `false`: behave exactly like GNU cat, also on a terminal (no rendering, pager, directory listing or terminal query). `true`: render as `scat` does, with all other settings applying as usual |

Messages say `cat:` either way. Explicit scat flags still work under the name `cat` (`cat -f FILE` renders, `cat --diagnose` reports). There are no other keys: with `render = true`, `[defaults]` and the rules control everything, as for `scat`.

## `[sqlite]`

| Key | Default | Meaning |
|---|---|---|
| `sample_rows` | `10` | rows shown per table; `0` shows the schema only |
| `max_tables` | `20` | tables shown in detail after the overview; `0` shows the overview only |

See [sqlite.md](sqlite.md).

## `[update]`

| Key | Default | Meaning |
|---|---|---|
| `channel` | `"stable"` | `"prerelease"` makes `scat --update` include pre-releases |

## `[[rule]]`

Rules change settings for matching files. Every matching rule applies, in file order, and later rules override earlier ones.

```toml
[[rule]]
match = { ext = ["md", "txt"] }        # prose without line numbers
line_numbers = false

[[rule]]
match = { name = "Jenkinsfile" }
language = "Groovy"

[[rule]]
match = { mime = "text/*", ext = "log" }
renderer = "text"                       # no highlighting for logs

[[rule]]
match = { ext = "csv" }
renderer = "raw"                        # print exactly like cat
```

`match` criteria (each takes a string or a list; all given criteria must match; an empty `match` matches every file):

| Criterion | Matches |
|---|---|
| `ext` | file name ending, case-insensitive; compound extensions work (`"tar.gz"`) |
| `name` | exact file name, e.g. `"Makefile"` |
| `mime` | detected type, with wildcards: `"text/*"`, `"image/png"`, `"*"` (or `"*/*"`, everything) |

Settings a rule can change:

| Key | Values |
|---|---|
| `renderer` | `"auto"`, `"code"` (highlight), `"text"` (no highlight), `"markdown"` ([rendered](markdown.md)), `"image"` (draw as an image, see [images.md](images.md)), `"audio"` (tags and waveform, see [audio.md](audio.md)), `"video"` (info view, see [video.md](video.md)), `"table"` (CSV/TSV table, see [data-files.md](data-files.md)), `"diff"` ([coloured diff](diffs-and-logs.md)), `"log"` ([coloured levels and timestamps](diffs-and-logs.md)), `"hex"` ([hex dump](rendering.md#hex-dump)), `"binary"` ([executable and library summary](binaries.md)), `"pdf"` (info and pages, see [pdf.md](pdf.md)), `"archive"` (summary and file tree, see [archives.md](archives.md)), `"document"` (docx, pptx, xlsx, odt, ... as markdown, see [office-documents.md](office-documents.md)), `"cert"` (certificate and key summaries, private keys hidden, see [certificates-and-keys.md](certificates-and-keys.md)), `"database"` (SQLite tables and rows, see [sqlite.md](sqlite.md)), `"diagram"` (mermaid, graphviz or PlantUML drawn as an image, see [diagrams.md](diagrams.md)), `"raw"` (exactly cat; also keeps compressed files compressed) |
| `language` | syntax name or extension, as with `-l` |
| `line_numbers` | `true` / `false` |
| `theme` | theme name |
| `tab_width` | number |

`scat --diagnose FILE` shows which rules matched a file and the resulting settings.

## Environment variables

| Variable | Same as |
|---|---|
| `SCAT_CONFIG` | `--config` |
| `SCAT_THEME` | `--theme` |
| `SCAT_COLOR` | `--color` |
| `SCAT_PROTOCOL` | `--protocol` |
| `SCAT_PAGING` | `--paging` |
| `SCAT_PAGER`, `PAGER` | the pager command, in that order (default `less -RFX`) |
| `SCAT_REPO` | GitHub `owner/repo` used by `--update` and `install.sh` (for forks) |
| `SCAT_INSTALL_DIR` | install directory for `install.sh` |
| `NO_COLOR`, `CLICOLOR`, `CLICOLOR_FORCE` | standard colour switches |
| `XDG_RUNTIME_DIR` | where terminal query answers are cached for 10 minutes; unset means no cache (see [terminal-detection.md](terminal-detection.md#cache)) |
