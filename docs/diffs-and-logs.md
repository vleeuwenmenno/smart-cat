# Diffs and logs

Two line-oriented colour views. Both stream, so `tail -f app.log | scat -l log` shows each line as it arrives, and both keep the normal line number gutter. With colours off the output is plain text.

## Diffs

Unified diffs (`git diff`, `diff -u`, `.patch` files) are coloured:

| Line | Style |
|---|---|
| `+` additions | green |
| `-` deletions | red |
| `@@ ... @@` hunk headers | cyan |
| `diff --git`, `index`, `---`, `+++`, `rename`, mode lines | bold |
| `\ No newline at end of file` | dim |

scat uses the line counts in each hunk header, so a deleted line that starts with `--` is shown as a deletion, not as a file header. Combined diffs (`@@@`) are coloured by the first character only. Word-level highlighting inside changed lines is not done.

Chosen automatically for `.diff` and `.patch` files and for text whose first lines look like a diff (`diff --git`, or `---` / `+++` / `@@ -` in a row). Force it with `-l diff` or `renderer = "diff"`.

## Logs

| Part | Style |
|---|---|
| leading timestamp: `2026-10-23T13:42:10.5Z`, `2026-10-23 13:42:10`, `[2026-10-23 13:42:10]`, `Oct 23 13:42:10`, `13:42:10` | dim |
| `FATAL`, `CRIT`, `PANIC`, `EMERG`, `ALERT` | bold red |
| `ERROR`, `ERR`, `SEVERE` | red |
| `WARN`, `WARNING` | yellow |
| `INFO`, `NOTICE` | green |
| `DEBUG`, `TRACE`, `VERBOSE` | dim |

The level is the first upper-case keyword near the start of the line, so prose such as "an error occurred" is left alone. A `level=warn` or `"level":"error"` field is recognised in any case; in JSON lines that field is the only thing coloured.

A line that already has escape sequences is shown as in any text file, following the `ansi` setting, without extra colours.

Chosen automatically for `.log` files, and for files without an extension (or `.txt`, `.out`, `.err`) and standard input when most of the first lines start with a timestamp. Force it with `-l log` or `renderer = "log"`. Use `renderer = "text"` to switch it off for `.log` files.
