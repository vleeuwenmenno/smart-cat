# PDF

In a terminal, `scat doc.pdf` prints an info header and draws page 1, fitted to the terminal, then says how to see more:

```
doc.pdf
  title      BANDPLAN AMATEUR
  author     Theo Vermeulen
  pages      34
  page size  216 x 279 mm (Letter)
  created    2016-07-17 18:54
  producer   Microsoft® Word 2016
  file       PDF 1.5, 559.2 KiB

[page 1 as an image]
page 1 of 34 — use --pages 2-4 for more
```

Piped output is the raw file, byte for byte, like `cat`.

## Options

| Option | Effect |
|---|---|
| `--pages SPEC` | pages to show: `1`, `1-3`, `2,5-7`, `4-` (to the end) or `all` |
| `--text` | show the text of the pages instead of drawing them |
| `-p`, `--plain` | no header, captions or footer |
| `-f` | render into a pipe; this gives the text version (`scat -f doc.pdf \| grep word`) unless `--protocol` asks for images |
| `--protocol` | how pages are drawn, as for [images](images.md) |

Pages beyond the end are skipped with a note on stderr (`no page 40 (the document has 34 pages)`). An invalid spec is a usage error. With several pages each gets a dim `page N` caption.

Images are drawn for at most `[pdf] max_pages` pages per file (default 20, so `--pages all` on a long document stops there with a note). Text output has no limit.

## Images or text

Pages are drawn with the method chosen for images (kitty, sixel, Unicode blocks; see [images.md](images.md)). The page is scaled to fit the terminal width and the height left after the header and footer, and to `[image] max_height` if set.

Text is shown instead:

- with `--text`,
- when the only image method left is ASCII (no colours, or a non-UTF-8 locale, or `--protocol ascii`), since ASCII art of a page is unreadable,
- with `-f` into a pipe.

Text comes from `pdftotext -layout` (poppler), which keeps columns and tables in place. Every line is sanitized like other text from files: control characters show as `^X`, escape sequences never reach the terminal. Without `pdftotext` scat prints the header and then `scat: doc.pdf: no text output: install poppler (pdftotext) for text output`.

## Header

Title, author, subject, page count, the first page's size in millimetres (with the paper name when it matches A3, A4, A5, Letter, Legal or Tabloid), creation date, producer (and creator when different), PDF version, encryption and file size. Empty fields are left out. All values come from the file and are sanitized.

## Encrypted files

- Encrypted with an empty user password (only an owner password, common for "no printing" or "no copying" restrictions): shown normally; the header says `encrypted (opened without a password)`.
- Needs a password: the header says `encrypted (needs a password)` and scat reports `encrypted PDF: a password is needed, pages not shown` (exit status 1). There is no password option.

## How pages are rendered

1. **hayro** (pure Rust, MIT/Apache-2.0) is built in. It parses the file, reads the metadata and rasterizes pages.
2. If hayro fails on a file or a page (unsupported feature, parse error, crash, timeout), **poppler** takes over when installed: `pdfinfo` for the header, `pdftoppm` for the page.
3. If both fail, scat reports `scat: doc.pdf: cannot read PDF: ...` or `page N: cannot render: ...`, exits with status 1 at the end, and goes on with the next file.

MuPDF is not used: its bindings are AGPL.

Typical timings (release build, 160x50 terminal, whole run including the header): a Word text page about 30 ms with Unicode blocks, 40 ms with kitty and 65 ms with sixel; a one-page Visio diagram about 100 ms with sixel. The info header alone takes about 3 ms, and text output with `pdftotext` about 20 ms.

## Sandbox

PDF files are untrusted input. Parsers can panic, loop or allocate without bound, and scat's release builds abort on panic. So scat never parses a PDF in its own process:

- hayro runs in a child process: scat starts its own binary (`/proc/self/exe`) with a hidden first argument, `--internal-pdf info FILE` or `--internal-pdf render FILE PAGE WIDTH HEIGHT`, and reads JSON or a PNG from its standard output.
- The poppler tools run the same way.
- Each child gets an address space limit of 1 GiB (`RLIMIT_AS`), 15 s of CPU time, no core dumps, `/dev/null` as standard input, and a 10 s wall clock timeout after which it is killed. At most 128 MiB of output is read back.
- Arguments are passed as an argv array; for the poppler tools the file name comes after `--`.
- A crash or timeout becomes an error message, never a crash of scat.

Standard input and pipes (`cat doc.pdf | scat -f -`) are first copied to a private temporary file (mode 0600 in a fresh 0700 directory, removed afterwards), because the helpers open the file by name. Inputs above 512 MiB are refused.

Not yet done: landlock and seccomp filters for the helper.

## Which files

Files whose content starts with `%PDF-` or that are detected as `application/pdf` use this view. To force it, or to opt out, use a rule:

```toml
[[rule]]
match = { ext = "ai" }
renderer = "pdf"     # or "raw" to print the bytes
```

## Config

```toml
[pdf]
pages = "1"        # default for --pages
max_pages = 20     # most pages drawn as images per file
```

`scat --diagnose doc.pdf` shows whether `pdftoppm` and `pdftotext` were found.
