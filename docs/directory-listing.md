# Directory listing

When scat renders (standard output is a terminal, or `--force-render`), a directory is listed instead of failing. On a terminal that can draw images, the listing looks like a file manager's icon view: a tile per entry with a thumbnail or icon and the name under it.

```sh
scat                      # no arguments, in a terminal: list the current directory
scat ~/Pictures           # tiles with thumbnails
scat src docs             # several directories: a "dir:" header before each
scat --all .              # include hidden (dot) entries
scat --style grid .       # names in columns, like ls
scat --long .             # one entry per line: type, size, age, name (same as --style long)
scat --tile-size 24x12 .  # bigger tiles
scat --no-thumbnails .    # tiles with icons only
```

## When it applies

| Situation | Behaviour |
|---|---|
| `scat`, stdin and stdout are terminals | list the current directory |
| `echo hi \| scat` (stdin not a terminal) | read stdin, like cat |
| `scat` with stdout piped | read stdin, like cat |
| `scat some/dir` when rendering | list it |
| `scat some/dir \| x` | `scat: some/dir: Is a directory`, exit 1, like cat |

Files and directories can be mixed; each is handled in argument order. An unreadable directory prints `scat: NAME: Permission denied`, sets exit status 1 and scat continues. Opt out in the [config](configuration.md): `no_args = "stdin"` disables the no-argument listing, `directories = "error"` makes directory arguments an error again.

## Styles

| Style | Looks like |
|---|---|
| `tiles` | a grid of tiles: thumbnail or icon, name centred below |
| `grid` | names in columns, like `ls` |
| `long` | one entry per line: type letter, size, age, name |
| `auto` (default) | `tiles` when images can be drawn, else `grid` |

`auto` picks tiles when standard output is a terminal, `TERM` is not `dumb` or `linux`, and images can be drawn with kitty graphics, sixel or coloured Unicode blocks (see [images.md](images.md)). It falls back to the grid with `--color=never`, on ASCII-only terminals, and for `-f` into a pipe. Choose a style for one run with `--style tiles|grid|long|auto`; `--long` is short for `--style long` and wins over `--style`. The default comes from `[list] style`.

## Order and names

- Directories first, then files; each group sorted case-insensitively with numbers compared by value (`file2` before `file10`).
- Hidden entries are not shown unless `--all` or `[list] show_hidden = true`.
- Names are coloured with `LS_COLORS` (see below).
- Grid: columns fill top to bottom and use the terminal width (the tty size, else `$COLUMNS`, else 80). Column widths use display width, so wide characters stay aligned. Directories get a trailing `/`. Symlinks are only coloured; `--long` shows `name -> target`.
- Tiles: tiles fill rows left to right, as many per row as fit the terminal width with two columns between them. A name longer than the tile is shortened with `…`, keeping a short extension (`a-very-long….png`); wide characters count as two columns.

## Tiles

Each tile is a box of `tile_width` x `tile_height` cells (default 16x8; `--tile-size WxH` for one run, 4 to 200 each) with the name on the line below. Thumbnails keep their aspect ratio, are centred horizontally and sit on the bottom of the box, so every picture rests on its name.

### Thumbnails

| File | Thumbnail |
|---|---|
| images (PNG, JPEG, GIF, WebP, BMP, ICO, TIFF, QOI, TGA, PNM, SVG) | the image; animations show their first frame |
| video (mp4, mkv, webm, mov, avi, ...) | a frame 2 s in, from `ffmpeg` (the first frame for shorter clips) |
| PDF | the first page, rendered by the same sandboxed helper as the [PDF view](pdf.md) |
| audio (mp3, flac, ogg, opus, m4a, ...) | the embedded cover art |

Files are recognised by extension, so listing a large directory does not read every file. Everything else gets an icon, and so does any file whose thumbnail cannot be made (no cover art, a broken image, no `ffmpeg`).

Thumbnails are drawn with the method chosen for images: kitty graphics, sixel, or Unicode half blocks. Sixel needs the terminal to report its cell size; without it the tiles use half blocks. `--protocol` changes the method for one run.

### Icons

Entries without a thumbnail get an icon in the same box, set with `[list] icons`:

| Value | Directories | Files |
|---|---|---|
| `"unicode"` (default) | a folder drawn with block characters in the directory colour | a coloured badge with the extension (`RS`, `TOML`, `EXE` for executables); colours group code, config, documents, archives, media |
| `"nerd"` | a Nerd Font folder glyph | a Nerd Font glyph per type, with the extension under it |
| `"none"` | nothing, only the name | nothing |

Nerd Font glyphs need a patched font, which scat cannot detect, so they are opt-in. Without colours the unicode icons become outlines (`╭──────╮`), and ASCII art (`+----+`) without a UTF-8 locale.

### Cache

Making a thumbnail can mean decoding a 20 MP photo or starting ffmpeg, so thumbnails are cached:

1. A thumbnail a file manager already made is used when it is fresh: scat looks in the freedesktop cache (`$XDG_CACHE_HOME/thumbnails/{large,x-large,normal,xx-large}/<md5 of the file:// URI>.png`) and checks that its `Thumb::MTime` matches the file.
2. Otherwise scat's own cache: `$XDG_CACHE_HOME/scat/thumbnails/` (default `~/.cache/scat/thumbnails/`), one PNG of at most 256x256 per file, named by a hash of the path, size and modification time. A changed file gets a new thumbnail. Files that have none (no cover art, undecodable) are remembered as an empty file. The directory is created with mode 0700 and the files with 0600.

`[list] cache = false` turns both off: thumbnails are made on every run and nothing is written.

Cache files are untrusted, even in the user's own cache directory:

- scat's cache directory is used only when it is a real directory (not a symlink) owned by you with mode 0700, inside a parent that others cannot write to. Otherwise nothing is read from or written to it.
- Every cache file is opened without following symlinks and without blocking (a FIFO in the cache cannot hang scat), and is read only when it is a regular file of at most 16 MiB. Files in scat's cache must be owned by you; freedesktop thumbnails may belong to someone else but must be regular files too.
- New entries are written to a fresh file (`O_EXCL`, `O_NOFOLLOW`, mode 0600) and renamed into place.

#### Pruning

After a tiles listing, at most once a day, scat prunes its own cache:

1. Entries not modified for 30 days are removed.
2. If the rest is larger than 256 MiB, the oldest are removed until it fits.

Only regular files owned by you, directly inside `$XDG_CACHE_HOME/scat/thumbnails`, are removed; symlinks, subdirectories and other files are left alone. The directory is opened once (`O_NOFOLLOW`) and every entry is looked at and removed relative to it. The time of the last prune is the modification time of `.pruned` in that directory; when it is less than a day old, the check costs one `lstat`. The freedesktop cache is never pruned by scat.

### Limits

Directories can hold thousands of files, so:

- At most `[list] max_thumbnails` (default 200) thumbnails per listing; later entries get icons.
- Thumbnails are made in parallel, on up to 8 threads.
- Images and PDFs larger than 64 MiB get an icon.
- ffmpeg and the PDF helper get 3 seconds per file.
- Images are decoded with the same limits as the image view (100 megapixels, 1 GiB of decoder memory); PDFs are parsed in the sandboxed helper.

With a warm cache, a directory of 100 images lists in about 30 ms (half blocks).

## Paging

The grid, the long view and tiles drawn with Unicode blocks are text, so they can go through the [pager](paging.md). Tiles with kitty or sixel thumbnails never do; a run that lists such a directory is not paged.

## Colours

When colours are on, `LS_COLORS` is honoured: the keys `di`, `ln`, `or`, `ex`, `fi`, `so`, `pi`, `bd`, `cd` and `*.ext` patterns. Values may only contain digits and `;`. Without `LS_COLORS`: directories bold blue, symlinks cyan, executables bold green, hidden entries dim.

## Safety

File names are untrusted. Control characters are shown as `^X` (a newline in a name is `^J`), and invisible formatting characters such as bidi overrides are replaced by `U+FFFD`, so a name cannot emit escape sequences or break the grid or the tiles. ffmpeg gets the file as a single `file:` argument, never through a shell.
