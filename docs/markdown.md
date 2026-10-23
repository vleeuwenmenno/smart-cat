# Markdown

In a terminal, `scat README.md` renders the document instead of printing its source: styled headings, wrapped paragraphs, lists, tables, highlighted code blocks and links. Piped output is still the file's exact bytes, like cat; `-f` renders into a pipe too (`scat -f README.md | less -R`).

Files ending in `.md`, `.markdown`, `.mdown` or `.mkd` are rendered. Standard input is rendered when the language is forced to markdown:

```sh
curl -s https://example.com/README.md | scat -l md
```

## What is rendered

| Element | Shown as |
|---|---|
| headings | bold, one colour per level; `#` and `##` get a full-width rule below (`━` / `─`), deeper levels keep their `###` marker |
| paragraphs | word-wrapped to the terminal width |
| emphasis, strong, strikethrough | italic, bold, struck through |
| inline code | its own colour, on a subtle background with 256 or 24-bit colour |
| code blocks | indented 4 columns, syntax highlighted (language from the fence, e.g. ` ```rust `) |
| block quotes | a `│` bar per level; GitHub alerts (`> [!NOTE]`, `[!TIP]`, `[!IMPORTANT]`, `[!WARNING]`, `[!CAUTION]`) get a coloured bar and label |
| lists | `•` `◦` `▪` by depth, numbers for ordered lists, a hanging indent when an item wraps |
| task lists | `☐` / `☑` |
| tables | box-drawing borders, column alignment from the `:--` / `:-:` / `--:` row |
| links | underlined text followed by the dimmed target in parentheses (left out when the text is the URL) |
| images | a dimmed `[image: alt text]` placeholder |
| horizontal rules | a full-width line |
| HTML | dimmed, as written; HTML comments are hidden; `<br>` breaks the line |
| footnotes | `[1]` at the reference; the notes are collected after a short rule at the end |
| YAML front matter | shown as a highlighted YAML code block |

Without UTF-8 in the locale the ASCII forms are used: `*` / `-` bullets, `[ ]` / `[x]`, `|` quote bars, `+-|` table borders, `=` / `-` rules.

With colours off (`NO_COLOR`, `--color=never`, or `-f` into a pipe without `--color=always`) the layout stays (indentation, bullets, wrapping, tables), no escape sequences are written, and inline code keeps its backticks so it stays recognisable.

## Width and wrapping

The width is the terminal's column count, else `$COLUMNS`, else 80, and never more than 100 columns, because longer lines are hard to read. Text inside quotes and lists wraps within the remaining width.

- A word longer than a line (a long URL) is split across lines.
- Code blocks are never wrapped or cut: long lines overflow and the terminal wraps them. This keeps every character visible and copyable.
- Tables keep their natural column widths when they fit. When they do not, the widest columns shrink step by step and cell text wraps inside them; when a body row wraps, rows get separators so they stay apart. A table with so many columns that even 4-column cells do not fit overflows.

## Links and hyperlinks

When colours are on and standard output is a terminal, link text is also an [OSC 8 hyperlink](https://gist.github.com/egmontkob/eb114294efbcd5adb1944c9f3cb5feda): terminals that support it (kitty, foot, ghostty, wezterm, iTerm2, GNOME Terminal, Windows Terminal) make it clickable, others ignore it. The target is percent-encoded first: spaces, control characters and non-ASCII bytes become `%XX`, so a link cannot smuggle an escape sequence. Links are not hyperlinked when writing to a pipe, or on `TERM=linux`/`dumb`.

## Safety

The document is untrusted. Every piece of text, code, HTML and link target goes through the same sanitizer as plain files (see [rendering.md](rendering.md#escape-sequences-in-files)): control characters are shown as `^X`, and escape sequences in the file are never passed through, not even SGR colours.

## Seeing the source

- `scat --raw README.md` prints the file exactly, like cat.
- `-l LANG` with any language other than markdown highlights the file as that language instead of rendering it.
- A [config rule](configuration.md) shows highlighted source every time:

  ```toml
  [[rule]]
  match = { ext = "md" }
  renderer = "code"
  ```

  `renderer = "markdown"` does the reverse and renders other files (say `ext = "txt"`) as markdown.

Line numbers (`-n`, `line_numbers`) and `-p` / `--plain` do not apply to rendered markdown; they apply when the source is shown.

## Limits

- Images are placeholders until image output lands (see the [roadmap](roadmap.md)).
- Files larger than `max_highlight_bytes` (16 MiB by default) are shown as source text instead of being rendered.
- Ordered list numbers are not right-aligned, so items 9 and 10 indent their wrapped lines differently.
- Math (`$...$`) and definition lists are shown as plain text.
