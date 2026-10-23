# Office documents

In a terminal, `scat report.docx` converts the document to markdown and draws it with the [markdown renderer](markdown.md), under a dim header line:

```
Word document (docx), 3 pages, 48.2 KiB

Quarterly report
━━━━━━━━━━━━━━━━━

Plain, bold and italic & a link (https://example.com/x)
...
```

Piped output is the raw file, byte for byte, like `cat`. `-f` renders into a pipe (`scat -f report.docx | grep budget`); `-p` drops the header line.

## Formats

| Files | What is shown |
|---|---|
| `.docx` `.docm` | headings, paragraphs, bullet and numbered lists, tables, bold, italic, strikethrough, hyperlinks |
| `.odt` | the same, from OpenDocument |
| `.pptx` `.pptm` | one `## Slide N` section per slide, in presentation order: title (as `###`), bullets, paragraphs, tables |
| `.odp` | the same, from OpenDocument |
| `.xlsx` `.xlsm` `.xls` `.ods` | one `## Sheet name` section per visible sheet with the cells as a table |

The header line names the type and, when cheap, the page count (from the document's metadata), slide count or sheet count, and the file size.

Spreadsheets show the first row as the table header. Dates and times are written as `2026-10-23` or `2026-10-23 14:30:00`, durations as `h:mm:ss`. Formulas show their cached value. Hidden sheets are skipped.

## Detection

A file is a document when its extension is one of the above and its first bytes match the container (zip for everything but `.xls`, which is an OLE file). A document without a useful name (standard input) is found by its MIME type. A `.zip` renamed to `.docx` without the document parts prints `scat: NAME: cannot read document: missing word/document.xml, not a valid document`.

To see the container instead, set a rule: `renderer = "hex"` (or `"raw"`), see [configuration.md](configuration.md). `renderer = "document"` forces this view.

## What is not shown

- Images are not drawn; they show as `[image]`.
- Headers, footers, footnotes, comments, tracked changes, speaker notes, charts and text boxes are left out.
- Merged cells are not merged.
- Only `http://`, `https://` and `mailto:` links are kept; others show just their text.

## Limits

Documents are untrusted input, so work is capped:

| Limit | Value |
|---|---|
| file size | 256 MiB |
| one XML part, decompressed | 64 MiB |
| all parts of a spreadsheet, as the zip declares them | 512 MiB |
| markdown produced | 8 MiB; the rest is replaced by a note |
| slides | 200, then `… N more slides` |
| sheets | 100 |
| table columns | 64, then `… N more columns` |
| rows per sheet | `[office] max_rows` (default 200), then `… N more rows`; `0` shows all |

Text from a document is stripped of control characters and escape sequences before it reaches the markdown, so a document cannot draw on or retitle your terminal.

A file that cannot be read prints `scat: NAME: cannot read document: REASON` on stderr, exit status 1, and `scat` goes on with the next file. Password-protected (encrypted) Office files are not zip files and report `not a zip file`.

## Paging

The converted document is text, so long documents go through the pager like markdown.

## Implementation

No external tools. The zip container is read with `zip`, the XML with `quick-xml` and spreadsheets with `calamine`, all pure Rust. The mcat project's `markdownify` crate was considered and not used: it reads and converts a whole file without caps on rows, slides or output, inlines images as base64, pulls in tar, encoding and archive handling scat already has elsewhere, and returns one markdown string with no way to bound the work.
