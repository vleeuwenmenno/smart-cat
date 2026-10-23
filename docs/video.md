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

`scat --play clip.mp4` plays the file with mpv; see [audio.md](audio.md#--play).
