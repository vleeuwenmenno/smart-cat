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
| `no_args` | `"list"` | `scat` with no arguments, run from a terminal: `"list"` lists the current directory, `"stdin"` reads stdin like cat |
| `directories` | `"list"` | directory arguments when rendering: `"list"` or `"error"` (cat's `Is a directory`) |

## `[list]`

| Key | Default | Meaning |
|---|---|---|
| `style` | `"grid"` | `"grid"` or `"long"` (`--long` for one run) |
| `show_hidden` | `false` | show dot entries (`--all` for one run) |

See [directory-listing.md](directory-listing.md).

## `[image]`

| Key | Default | Meaning |
|---|---|---|
| `protocol` | `"auto"` | `"auto"`, `"kitty"`, `"sixel"`, `"blocks"`, `"sextants"` or `"ascii"`; see [images.md](images.md) |
| `max_height` | unset | maximum image height in rows; unset means the terminal height minus 4 (at least 10) |
| `animate` | `true` | play animated GIF/WebP/PNG once; `false` shows the first frame |

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
| `mime` | detected type, with wildcards: `"text/*"`, `"image/png"`, `"*"` |

Settings a rule can change:

| Key | Values |
|---|---|
| `renderer` | `"auto"`, `"code"` (highlight), `"text"` (no highlight), `"markdown"` ([rendered](markdown.md)), `"image"` (draw as an image, see [images.md](images.md)), `"audio"` (tags and waveform, see [audio.md](audio.md)), `"video"` (info view, see [video.md](video.md)), `"raw"` (exactly cat) |
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
| `SCAT_REPO` | GitHub `owner/repo` used by `--update` and `install.sh` (for forks) |
| `SCAT_INSTALL_DIR` | install directory for `install.sh` |
| `NO_COLOR`, `CLICOLOR`, `CLICOLOR_FORCE` | standard colour switches |
