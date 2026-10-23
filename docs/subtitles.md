# Subtitles

Rendering applies in a terminal only. When output is piped or redirected, a subtitle file is printed byte for byte like `cat`.

`scat movie.srt` shows an overview, then the cues. Formats: SubRip (`.srt`), WebVTT (`.vtt`), Advanced SubStation Alpha and SubStation Alpha (`.ass`, `.ssa`). A file is picked by its extension, or by starting with `WEBVTT` or `[Script Info]`. `-l srt|vtt|ass|ssa` forces a format. MicroDVD `.sub` is not supported (it needs a frame rate).

## Overview

A table with: format, cue count, first and last timestamp, total span, language, and for ass the title and style count.

- **language** comes from the VTT `Language:` header or the ass `[Script Info]`, else from a file name like `movie.en.srt` or `movie.pt-BR.srt` (marked as a guess).
- **encoding** appears when the file is not UTF-8. UTF-16 with a byte order mark is converted; other non-UTF-8 text is decoded as Windows-1252 (Latin-1 plus the usual punctuation), lossily.
- **warnings**, each shown only when its count is above zero:
  - `overlapping`: cues that start before the previous cue ends;
  - `out of order`: cues that start before the previous cue starts;
  - `empty`: cues without text;
  - `bad duration`: cues that end at or before their start;
  - `too long`: cues longer than 10 s;
  - `fast reading`: cues above 25 characters per second.

## Cues

```
movie.en.srt
  format        SubRip (srt)
  cues          3
  first         00:00:01.000
  last          00:00:35.000
  span          34.000 s
  language      en (from the file name)
  overlapping   1 cue start before the previous one ends
  too long      1 cue last over 10 s

   1 │ 00:00:01.000 → 00:00:03.500  Hello world
     │                              second line
   2 │ 00:00:03.000 → 00:00:04.000  Overlaps the first
```

- The times are dim; multi-line cues continue under the text.
- Tags are removed. `<i>` and ass `\i1` show as italic when colours are on. srt `{\an8}` blocks, VTT `<c.x>`, `<v Name>` and timestamp tags are dropped; `&amp;` style entities are decoded.
- ass: `{\...}` override blocks are removed, `\N` is a line break, `\h` a space. The style (when not `Default`) and the actor show dim in brackets before the text. `Comment:` lines are skipped.
- VTT: cue settings (`align:start`) show dim in brackets before the text. `NOTE`, `STYLE` and `REGION` blocks are skipped.
- Line numbers (on by default, `-n`, `line_numbers`) number the cues instead of lines. `--plain` turns them off.
- All text is sanitized like any file: control and escape characters show as `^X`.

## Limits

- At most 16 MiB of a file are read; the overview then says so.
- Only the first 500 cues are listed (`[subtitles] max_cues`); the rest are counted, and the list ends with `… N more cues`.

## Not a subtitle file

A file that does not parse (no cues, a `.vtt` without the `WEBVTT` header, no `[Events]` section) prints a note on stderr and is shown as plain text (or a hex dump when binary). `renderer = "code"` for a rule shows the source highlighted instead.

```toml
[[rule]]
match = { ext = "srt" }
renderer = "code"
```
