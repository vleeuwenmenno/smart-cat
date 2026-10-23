# Audio

In a terminal, `scat song.flac` prints an info view for the file and exits, like `cat`. `scat --play song.flac` plays it with mpv. Piped (`scat song.flac | x`) or with `--raw`, the output is the file's bytes, unchanged.

```text
01 - rez+kenet - unreeeal superhero 3.mp3
  title        unreeeal superhero 3
  artist       rez+kenet
  album        Essential Keygen Music
  track        1
  duration     2:07
  format       MPEG audio
  codec        MPEG Audio Layer 3
  bitrate      158 kbps
  sample rate  48 kHz
  channels     2 (stereo)
  size         2.4 MiB

⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⢀⣠⣄⣤⣦⣴⣤⣠⣤⣤⣦⡀⣀⣠⣤⣦⣦⣤⣤⣦⣴⣤⣆⣤⣤⣦⣼⣶⣦⣤⣄⣦⣼⣤⣦⣤⣀⠀⠀⠀⠀⠀⢀⠀⡀⢀⠀⡀⢀
⣀⣀⣠⣤⣀⣴⣦⣦⣶⣶⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣇⣼⣤⣇⣿⣸⣷⣷⣾⣶⣷⣼⣷⣷
⠉⠉⠙⠛⠉⠻⠟⠟⠿⠿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⡏⢻⠛⡏⣿⢹⡿⡿⢿⠿⡿⢻⡿⡿
⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠈⠙⠋⠛⠟⠻⠛⠙⠛⠛⠟⠁⠉⠙⠛⠟⠟⠛⠛⠟⠻⠛⠏⠛⠛⠟⢻⠿⠟⠛⠋⠟⢻⠛⠟⠛⠉⠀⠀⠀⠀⠀⠈⠀⠁⠈⠀
```

## What is shown

The title line is the file name. The table shows whatever the file has, and skips empty rows:

| Row | Source |
|---|---|
| title, artist, album, album artist, genre | tags |
| track, disc | tags, as `n/total` when the total is known |
| date | recording date, year or release date tag |
| comment | first line only, marked with `…` when there is more |
| duration | `m:ss`, or `h:mm:ss` from an hour on |
| format | container, e.g. `FLAC`, `MPEG audio`, `MP4 audio`, `Ogg Vorbis` |
| codec | as reported by the decoder, e.g. `Advanced Audio Coding`, `Apple Lossless Audio Codec` (only when the waveform decoder can read the file) |
| bitrate, sample rate, channels, bit depth | stream properties |
| size | file size |
| cover art | `yes (jpeg, 500x500)` when a picture is embedded; the picture itself will be drawn once image output lands |

Tag values come from the file and are untrusted: control characters and escape sequences are shown as `^X`, never sent to the terminal. Lyrics are not shown.

If the tags cannot be read (a damaged file), the table falls back to the detected type, the size and a `tags: unreadable (...)` row.

## Formats

Tags and properties come from [lofty](https://crates.io/crates/lofty): MP3, FLAC, Ogg Vorbis, Opus, Speex, MP4/M4A (AAC, ALAC), WAV, AIFF, WavPack, Monkey's Audio, Musepack and raw AAC (ADTS).

Files are recognised by content (`audio/*` MIME type). WavPack, Monkey's Audio, Musepack, Speex and raw AAC are not recognised by content, so for those the extension (`.wv .ape .mpc .spx .aac`) decides. A [config rule](configuration.md) with `renderer = "audio"` forces the view for any file; `renderer = "raw"` turns it off.

## Waveform

Below the table, a waveform of the whole file: the peak amplitude per column, scaled so the loudest peak reaches full height, mirrored around a centre line, 4 rows tall and as wide as the terminal (window size, else `$COLUMNS`, else 80).

- With a UTF-8 locale it is drawn with braille characters (2x4 dots per cell); otherwise with ASCII `#` (full), `:` (half) and `.` (silence).
- It is cyan when colours are on.
- Decoding uses [symphonia](https://crates.io/crates/symphonia): MP3, FLAC, Ogg Vorbis, AAC and ALAC in MP4, WAV and AIFF (PCM and ADPCM). Opus, WavPack, Monkey's Audio and Musepack have no decoder there, so those files show the table only.
- Decoding stops after 1 second. A 5-minute MP3 takes about 0.1 s in a release build, so this only matters for files over roughly 25 minutes (podcasts, mixes); the waveform then covers the decoded part, takes a matching share of the width, and a line says `waveform covers the first 25:24 only`.
- If decoding fails, the table is shown without a waveform.

## Standard input

lofty and symphonia need to seek, so audio from standard input or a pipe is read into memory, up to 64 MiB. Larger input gives `scat: -: audio input over 64.0 MiB; audio info needs a file (use --raw for bytes)`.

## `--play`

`scat --play FILE...` plays each audio or video file in turn with [mpv](https://mpv.io), in the foreground, with the terminal handed over so mpv's keys work (space pauses, `q` ends the file, arrows seek). scat links no audio library.

| Input | mpv command |
|---|---|
| audio file | `mpv --no-video --term-osd-bar -- FILE` |
| video file | `mpv --vo=kitty --term-osd-bar -- FILE`; `--vo=sixel` when the terminal answers the sixel query, `--vo=tct` (coloured text) when it supports neither |
| standard input (`-` or no FILE) | as for video, with `-` as the file; the type is not checked |

- The type is checked by content: a file that is not `audio/*` or `video/*` gives `scat: NAME: not an audio or video file (TYPE); --play needs media` and the next file is played.
- Without mpv: `scat: NAME: --play needs mpv (not found on PATH)`, exit status 1.
- Ctrl+C stops mpv and scat (exit status 130). Any other mpv failure is reported as `scat: NAME: mpv exited with status N` and gives exit status 1.
