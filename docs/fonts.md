# Fonts

Rendering applies in a terminal only. When output is piped or redirected, a font file is printed byte for byte like `cat`.

`scat Inter.ttf` shows what is inside a font instead of its bytes: an info table for every face, then a specimen image set in the font.

```sh
scat Inter.ttf                    # table and specimen
scat NotoSansCJK-Bold.ttc         # every face of a collection gets its table
scat font.woff2                   # web fonts too
scat --protocol=ascii Inter.ttf   # the table only
scat --raw Inter.ttf | xxd        # the bytes
```

## Formats

| Format | Detected by | Notes |
|---|---|---|
| TrueType, OpenType (`.ttf`, `.otf`) | the sfnt header (`00 01 00 00`, `OTTO`, `true`) with a sane table count | TrueType, CFF and CFF2 outlines |
| collections (`.ttc`, `.otc`) | `ttcf` | every face is listed (up to 64); the specimen shows the first |
| WOFF (`.woff`) | `wOFF` | tables are inflated with zlib |
| WOFF2 (`.woff2`) | `wOF2` | the brotli stream is decoded and the `glyf`/`loca` and `hmtx` transforms are reversed, so WOFF2 fonts get the same table and specimen as the font they were made from; WOFF2 collections work too |

Detection looks at the content, not the extension. `renderer = "font"` in a [rule](configuration.md) forces the view; a file that is not a font then gives an error.

## The info table

```
AdwaitaSans-Regular.ttf
  format        OpenType
  family        Adwaita Sans
  style         Regular
  full name     Adwaita Sans
  PostScript    AdwaitaSans
  version       Version 4.001;git-9221beed3
  designer      Rasmus Andersson
  manufacturer  rsms
  weight        400 (Regular)
  width         5 (Normal)
  italic        no
  monospace     no
  axes          opsz 14–32, default 14
                wght 100–900, default 400
  instances     9
  outlines      TrueType
  glyphs        2,937
  units/em      2048
  characters    2,852
  coverage      Private use 745 · Latin 708 · Punctuation & symbols 448 · Greek 338 · Cyrillic 251
  features      aalt calt case ccmp cpsp cv01 cv02 cv03 cv04 cv05 cv06 cv07 cv08 cv09 cv10 cv11 cv12 cv13 cv14 dlig dnom frac kern locl mark +19 more
  license       This Font Software is licensed under the SIL Open Font License, Version 1.1. This license is available with a FAQ at: ht…
  license URL   http://scripts.sil.org/OFL
  file size     859.2 KiB
```

- **Names** come from the `name` table, English first: the typographic family and style (IDs 16/17) when present, else IDs 1/2. Designer, manufacturer and license show their first line, cut at 100–120 characters.
- **weight** and **width** are the `OS/2` classes; **italic** follows `OS/2` (italic or oblique) and the italic angle; **monospace** follows `post.isFixedPitch` or equal advance widths.
- **axes** lists each variation axis as `tag min–max, default d`; **instances** counts the named instances.
- **outlines** is `TrueType`, `CFF (PostScript)`, `CFF2`, or none for bitmap-only fonts; **colour** lists `COLR`, `SVG`, `CBDT`, `sbix` and `EBDT` tables when present.
- **coverage** counts the characters in the `cmap` per script, roughly by Unicode block (Latin, Greek, Cyrillic, Arabic, Hebrew, Devanagari, Thai, Hangul, Kana, CJK, emoji, box drawing, private use, …), most first, up to 8; scripts with fewer than 3 characters are left out.
- **features** lists the `GSUB` and `GPOS` feature tags, sorted, up to 24.

For a collection the first table holds the format, the number of faces and the file size, followed by one table per face (`face 0: Noto Sans CJK JP Bold`).

## The specimen

When images can be drawn (kitty graphics, sixel or Unicode blocks, see [images.md](images.md)), a specimen follows the table:

1. the font's full name, when the font has glyphs for it,
2. a pangram at 48, 32 and 18 px (`The quick brown fox jumps over the lazy dog`),
3. `ABCDEFGHIJKLMNOPQRSTUVWXYZ abcdefghijklmnopqrstuvwxyz 0123456789 !@#$%&*()`,
4. for monospaced fonts, three lines of code (`0O 1lI|`, operators, brackets),
5. for variable fonts with a `wght` axis, `Handgloves` at the lightest and the heaviest weight,
6. a line in up to two other scripts the font covers well (40 characters or more).

Fonts without Latin letters show a sample in their main script instead (Arabic, Hebrew, Cyrillic, Greek, Devanagari, Bengali, Tamil, Thai, Hangul, Kana, CJK, …), shaped right to left where the script needs it.

The specimen is **dark text on a light paper card**, whatever the terminal background. It looks the same everywhere and needs no terminal background query. Variable fonts are drawn at their default instance. On kitty and sixel terminals the nominal sizes are scaled by the cell height (16 px cells draw at 1x, up to 3x), so text stays sharp on HiDPI screens; the image is never wider than the terminal.

The text is laid out by resvg: scat builds an SVG document with `<text>` elements, usvg shapes it with harfrust (kerning, ligatures, complex scripts) and converts the glyphs to paths, and resvg fills them. The font database holds only the face shown, and the font resolver always returns it, so no system font is loaded and nothing is substituted: a character the font lacks shows as its `.notdef` glyph. These crates were already in scat for SVG images, so the specimen adds no new rasteriser.

No specimen is drawn when:

- only ASCII art is possible (`--protocol=ascii`, `--color=never`, a non-UTF-8 locale): the table alone is printed,
- output is piped with `-f` and no `--protocol`,
- the font has no outlines resvg can draw (colour bitmap fonts such as Noto Color Emoji): a dim `no specimen: ...` line explains why.

## Paging

The table alone is text and can go through the [pager](paging.md). When a specimen will be drawn the run is not paged, like any other image output.

## Safety

Font files are untrusted. All parsing, shaping and drawing runs in a helper process (`scat --internal-font`, the same mechanism as the [PDF helper](pdf.md)): the font is passed on standard input, and the child has a 1 GiB address space limit, a CPU limit and a 10 second timeout (3 seconds for directory thumbnails). A parser that panics or loops only ends the helper; scat prints `scat: NAME: cannot read font: ...` and goes on with the next file. The table and the specimen run in separate helpers, so a font that breaks the shaper still gets its table, with `no specimen: the parser crashed (signal 6)` below it.

- Font files over 128 MiB are not read; a WOFF or WOFF2 file may expand to at most 256 MiB.
- Every string from the font (names, license, tags) is sanitized before printing: control characters show as `^X` and escape sequences never reach the terminal.
- Text placed in the specimen's SVG is XML-escaped.

## Directory listings

In [tiles](directory-listing.md), fonts get a thumbnail with a big `Aa` set in the font (or two characters of its main script).
