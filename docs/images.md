# Images

In a terminal, `scat photo.png` draws the image, fitted to the terminal, then prints a dim caption:

```
photo.png  1920x1080 PNG, 1.2 MiB
```

When the output is piped or redirected, scat prints the file's bytes like `cat` does. `-f` (`--force-render`) draws the image into the pipe anyway. In that case scat sends no terminal queries and uses text blocks, or ASCII when colours are off.

## Formats

PNG (including APNG), JPEG, GIF, WebP, BMP, ICO, TIFF, QOI, PNM (`pbm pgm ppm pam`), TGA and SVG.

Raster types are detected from the file's content. QOI, PNM and TGA have no reliable signature, so scat goes by their extension. SVG is recognised by the `.svg` extension or by a text file that starts with `<svg` (an XML declaration, comments and a doctype may come first). HTML that merely contains an `<svg>` element is not treated as an image.

A `[[rule]]` with `renderer = "image"` forces any file through the image view; `renderer = "raw"` prints images as bytes.

## Output methods

With `--protocol auto` (the default), scat uses the first method in this list that the terminal supports:

| Method | `--protocol` | Used when |
|---|---|---|
| kitty graphics protocol | `kitty` | the terminal answers the kitty graphics query (kitty, ghostty, wezterm, konsole) |
| sixel | `sixel` | the terminal reports DA1 attribute 4 (foot, xterm, wezterm, konsole, Windows Terminal) |
| Unicode half blocks `▀` | `blocks` | UTF-8 locale and colours on (alacritty, GNOME Terminal/VTE, Linux console with UTF-8) |
| Unicode sextants (2x3 per cell) | `sextants` | only when asked for: many fonts lack U+1FB00 |
| ASCII ramp ` .:-=+*#%@` | `ascii` | no UTF-8 locale, or colours off |

The answers come from the [terminal query](terminal-detection.md), which runs only when an image is about to be drawn. The per-terminal support table is in [research.md §3.2](research.md#32-support-matrix-2026).

Notes per method:

- **kitty**: the image is sent as PNG inside the escape sequences (`t=d`, `f=100`), base64 in 4096-byte chunks, with replies suppressed (`q=2`). This works over SSH. An unscaled PNG file is sent as is; anything else is scaled and re-encoded as a fast PNG.
- **sixel**: scat quantises the image to at most 256 colours with median cut, dithers it with Floyd–Steinberg, and leaves transparent pixels unpainted. The height is rounded down to a multiple of 6 pixels so the image ends on a whole sixel band.
- **Blocks, sextants and ASCII**: colours follow the detected colour depth (24-bit, 256 or 16). Pixels under 50% alpha show the terminal background. Other partly transparent pixels are blended onto the background colour the terminal reported; when it reported none (or was not asked, as with `--protocol=blocks`), they keep their own colour, since blending onto a guessed black looks wrong on light terminals. On a light background, the ASCII ramp is inverted.
- **tmux**: sixel is used only if tmux itself reports sixel support (DA1 is answered by tmux). Pass-through of the kitty protocol is not supported, so inside tmux a kitty terminal gets text blocks.
- `--color=never` and `NO_COLOR` turn blocks and sextants into ASCII. They do not affect kitty or sixel.

`scat --diagnose` shows which method was picked and why: the query result, the setting, and where that setting came from.

## Size

- Width: at most the terminal width. scat reads the width from stdout's window size, then `$COLUMNS`, then falls back to 80.
- Height: at most `[image] max_height` rows. The default is the terminal height minus 4, and at least 10.
- The aspect ratio is kept, using the cell size in pixels that the terminal reported. Without a report, scat assumes 8x16 pixel cells (a 1:2 cell).
- kitty and sixel never enlarge an image beyond its pixel size, so small icons stay small.
- Text methods enlarge small images only up to the larger of: twice the cells the image would cover with kitty or sixel, and one sub-pixel per image pixel (at most 32 columns by 16 rows). A 16x16 icon becomes 16x8 cells of half blocks instead of filling the screen.
- SVGs are rendered by resvg at the final pixel size, and are allowed to grow up to 4 times their intrinsic size. Text in SVGs uses the system fonts, which are loaded only when the SVG contains `<text`. `<image>` elements that point to other files are not loaded. Embedded `data:` images still work.

### Cell size, HiDPI and sixel

A sixel image covers one text column per started cell width and one row per started cell height, in the pixels the terminal reports. scat sizes sixel images in whole 6-pixel bands and counts the rows from the rounded height, so the image ends on the row scat planned.

foot on Hyprland, measured with cursor position reports (`CSI 6 n`) and screenshots of the window:

| Monitor scale | `CSI 16 t` cell | `TIOCGWINSZ` / `CSI 14 t` | 180x390 px sixel covers |
|---|---|---|---|
| 1 | 9x20 | agrees (cells x cell size) | 20 columns, 20 rows (cursor ends on row 20) |
| 1.25 (fractional) | 11x25 | agrees | 17 columns, 16 rows |
| a new window that has not settled yet | 18x39 | agrees, but for a much larger window | |

foot reports cells and draws sixel pixels in the same buffer pixels, at any scale, so the sixel aspect parameters (`P1`, the `"1;1` raster attributes) do not matter there. The trap is the first moments of a new window: foot answers with scale-2 metrics (18x39 instead of 9x20) until the compositor tells it the real scale. An answer cached from that moment made every later image twice as large as the cells scat planned for it.

The [terminal query cache](terminal-detection.md) therefore records the window size (`TIOCGWINSZ`) its answer was made for and is only used while the window size is unchanged; a font zoom, a move to a monitor with another scale or a window that settled since all lead to a fresh query. kitty places images in cells (`c=`/`r=`), so this affects only how many pixels are sent, never the cells covered.

A 4000x3000 JPEG takes about 65 ms in a release build with text blocks, 95 ms with kitty, and 110 ms with sixel, all including decoding.

## Animation

Animated GIF, WebP and PNG play **once** on a terminal, then the last frame stays on screen with a caption such as `anim.gif  100x60 GIF, 24 frames, 12 KiB`.

- Each frame is drawn in place inside synchronized output (`CSI ? 2026 h/l`), with the cursor hidden. Frames are decoded one at a time, just before they are shown. Frame delays are respected, with a minimum of 20 ms.
- kitty: each frame replaces the previous image under a fixed image and placement id. scat does not use kitty's native animation (`a=f`/`a=a`), so playback works the same way on every method and leaves no animation running in the terminal after scat exits.
- Playback stops after 60 seconds at most, because `cat` must terminate.
- Ctrl-C stops playback, shows the cursor again, moves below the image and exits with status 130.
- `--no-animate` or `animate = false` shows only the first frame. Output that is not a terminal (`-f` into a pipe) always gets only the first frame.

## Errors and limits

- A corrupt image prints `scat: NAME: cannot decode image: ...`. scat then goes on with the next file and exits with status 1.
- Images with more than 100 megapixels are refused based on their header, before any decoding. Decoder allocations are capped at 1 GiB, and files larger than 512 MiB are not read for decoding.
- Only the encoded image data and scat's own control sequences reach the terminal. File names in captions are sanitized.

## Settings

| Flag | Environment | Config | Meaning |
|---|---|---|---|
| `--protocol METHOD` | `SCAT_PROTOCOL` | `[image] protocol` | `auto`, `kitty`, `sixel`, `blocks`, `sextants`, `ascii` |
| `--no-animate` | | `[image] animate = false` | first frame only |
| | | `[image] max_height` | maximum height in rows |
| `-p` | | | no caption |

## In other views

The same output methods draw [audio cover art](audio.md#cover-art), [video poster frames](video.md#poster-frame) and [local images in markdown](markdown.md#images).

## For other renderers

`src/image/mod.rs` exposes:

- `image::show(out, &screen, &DynamicImage, &ShowOptions { max_cols, max_rows, caption, .. })` draws an image and a caption.
- `image::draw` and `image::draw_encoded` (from file bytes, SVG included) return a `Drawn`: the encoded bytes plus the cells covered, to place yourself.
- `image::write_beside(out, &drawn, &lines)` lays text out to the right of an image; check `image::can_write_beside(&screen)` first and draw with `pin_rows: true`.
- `image::thumbnail(bytes, svg, max_side)` decodes with the image view's limits and scales down to `max_side` pixels (directory tiles).
- `image::size::thumbnail_box` gives the size limits used for cover art and poster frames.

`Context::image_screen()` in `render.rs` supplies the `Screen`: the chosen method, the size limits and the colours. It is detected once per run.
