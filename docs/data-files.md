# Data files

Rendering applies in a terminal only. When output is piped or redirected, these files are printed byte for byte like `cat`.

## CSV, TSV and PSV: tables

`.csv`, `.tsv` and `.psv` files are drawn as a box table:

```
┌───┬─────────────┬─────┬───────┐
│   │ name        │ qty │ price │
├───┼─────────────┼─────┼───────┤
│ 1 │ widget      │   3 │  1.50 │
│ 2 │ gadget, big │  12 │    20 │
└───┴─────────────┴─────┴───────┘
```

- The first record is the header (bold). A row-number gutter (dim) follows the `line_numbers` setting; `-p` hides it.
- Delimiter: `.tsv` is tab, `.psv` is `|`. For `.csv` scat looks at the first 8 KiB and picks the one of `,` `;` tab `|` that splits records into the most consistent number of fields; comma wins ties. Detection only runs for these extensions (or `-l csv`); other text files are never turned into tables.
- Quoted fields follow RFC 4180 (`"a, b"`, `""` for a quote, line breaks inside quotes). A line break inside a cell shows as `↵`.
- Columns where every non-empty cell below the header is a number are right-aligned.
- The table is fitted to the terminal width. The widest columns shrink first and long cells end in `…`; rows never wrap. If the columns cannot fit even at 2 characters, the last ones are left out and the footer says `… 3 more columns`.
- Only the first 200 data rows are shown. The rest are counted while streaming and reported in a dim footer, `… 12,345 more rows` (counting stops after 64 MiB, shown as `12,345+`). Change the limit with `[table] max_rows` (`0` shows every row, which holds them in memory).
- Text from the file is sanitized like everywhere else: control characters show as `^X`.
- Opt out with a rule: `renderer = "code"` shows the highlighted source instead.

## JSON: minified files are pretty-printed

JSON that is minified or on one line becomes readable, then goes through the normal highlighter (line numbers, themes). Applies to `.json` files (or `-l json`) that are

- an object or array with no newline in the first 4 KiB, or lines averaging over 200 bytes, and
- valid JSON, and under 16 MiB.

A one-line file shorter than 80 bytes is left as is. Already indented JSON keeps the plain highlighted view, and so does invalid JSON (no partial output, no error).

The text is re-indented (2 spaces), not re-serialised: key order, duplicate keys, number spelling (`1.0`, `1e3`, `12345678901234567890`) and string escapes (`é`) come out exactly as written. Empty `{}` and `[]` stay on one line. `renderer = "text"` or `"raw"` in a rule skips the reformatting.

JSON Lines (`.jsonl`, `.ndjson`) keep the current view: one record per line is already readable.

## YAML and TOML

No change: they are highlighted source, which is the readable form already. A tree view would hide comments and ordering without adding information.
