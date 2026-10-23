# Video

In a terminal, `scat clip.mp4` draws a poster frame, prints an info view and exits, like `cat` would. Piped output is the raw file, byte for byte.

```
clip.mp4
  format    QuickTime / MOV
  duration  0:22
  size      806.4 KiB
  bitrate   300 kbps
  encoder   Lavf63.1.101
  video     h264 (High), 1100x720, 60 fps, yuv420p
```

## What is shown

- Title line: the file name.
- Container (long name), duration (`m:ss` or `h:mm:ss`), file size, overall bitrate.
- Format tags `title`, `creation_time` and `encoder`, when present. Tag text is sanitized like all text from files, so escape sequences never reach the terminal.
- One line per stream:
  - `video`: codec and profile, size, frame rate, pixel format, `HDR10` or `HLG` when the transfer function says so
  - `audio`: codec, sample rate, channel layout, bitrate, language
  - `subtitle`: codec, language, `"title"`
  - `cover`: embedded cover art (codec, size)

## ffprobe

Details come from `ffprobe` (part of ffmpeg), found on `PATH`. scat runs it with an argument list and the `file:` prefix, so odd file names (spaces, colons, a leading dash) are safe. It is killed after 10 seconds.

Without ffprobe, or when ffprobe cannot read the file, scat still prints a short table (MIME type, size) plus a dim hint line: `install ffmpeg (ffprobe) for stream details`, `ffprobe could not read this file` or `ffprobe timed out reading this file`. These are not errors; the exit status stays 0.

## Poster frame

When `ffmpeg` is on `PATH` and ffprobe found a video stream (not only cover art), scat takes one frame at 10% of the duration (the first frame when the duration is unknown) and draws it above the table with the [image output](images.md) method of the run: at most 80 columns wide and 20 rows high, and never more than a third of the terminal height.

The command is `ffmpeg -nostdin -v error -ss T -i file:/abs/path -an -sn -frames:v 1 -f image2pipe -vcodec png -`, run with an argument list. It is killed after 10 seconds, its output is capped at 64 MiB, and any failure just leaves the poster out. Without ffmpeg, the table is shown alone.

ASCII output (no UTF-8 locale, or colours off) and terminals under 12 rows get no poster. `-f` into a pipe without `--color=always` is ASCII.

## Which files

Files detected as `video/*` (mp4, mkv, webm, mov, avi, ...) use this view automatically. To force it for other files, or to opt out, use a rule:

```toml
[[rule]]
match = { ext = "ts" }
renderer = "video"   # or "raw" to print the bytes
```

`--raw` or piping prints the bytes.

## Standard input

ffprobe needs a path, so `scat -` and `cmd | scat` cannot show the info view. scat prints `scat: -: video info needs a file (use --raw for bytes)` to stderr and exits with status 0, like the binary file notice.

## Playback

`scat --play clip.mp4` plays the file with mpv inside the terminal; see [audio.md](audio.md#--play) for the common rules (type check, errors, Ctrl+C). The video output follows the terminal: `--vo=kitty` for the kitty graphics protocol, `--vo=sixel` for sixel, `--vo=tct` (coloured text) otherwise. The command is:

```
mpv --vo=VO --quiet --msg-level=all=error --video-align-x=-1 --video-align-y=-1 -- FILE
```

- `--quiet --msg-level=all=error`: no status line and no messages below errors. mpv writes its status line (`V: 00:00:07 / 00:00:21`) and log messages to the same terminal as the picture, in the middle of the sixel or kitty escape stream. The stream then ends early and the rest of the image data is printed as text (rows like `A?A!5?@A??A?...`) above and below the video. mpv's own manual recommends this for the sixel and tct outputs. Seek with the arrow keys or the on-screen controls; there is no `--term-osd-bar`.
- `--video-align-x=-1 --video-align-y=-1`: the picture sits in the top left corner instead of the middle. The kitty and sixel outputs draw the on-screen controls into the picture, but mpv maps the mouse from terminal cells to pixels counted from the top left of the whole terminal. With a centred picture the mouse is off by the black border above and left of the video, so the controls react while the pointer is far above the seek bar. With the picture in the corner, both use the same origin and the pointer lands within half a cell.
- No sizes are passed. mpv reads the cell grid and pixel size from the terminal (`TIOCGWINSZ`) on every frame and every mouse event, so a resize, a font zoom or a move to a monitor with another scale is followed. Sizes from scat's own capability cache would go stale in exactly those cases.
- mpv's defaults do the rest: the alternate screen (the shell output is back after mpv quits), hidden cursor, mouse reporting on while playing and off on exit.

Known limitations:

- The mouse position is the centre of a terminal cell, so it is accurate to half a cell. mpv only reads button presses and motion in X10 mouse encoding, which stops at column and row 223; in wider terminals the mouse does not work past that column.
- `--vo=tct` draws no on-screen controls and centres the picture itself; use the keyboard.
- An error mpv prints in the middle of playback (a damaged stream) can still break one frame of the picture. Errors before playback starts, such as an unreadable file, are shown as usual.
- With `-` (standard input) the type is unknown, so audio on standard input also gets these options and plays without the status line.
