# Jupyter notebooks

Rendering applies in a terminal only. When output is piped or redirected, a notebook is printed byte for byte like `cat`.

`scat analysis.ipynb` shows the notebook instead of its JSON. A file is shown as a notebook when its name ends in `.ipynb` (or with `renderer = "notebook"` in a [rule](configuration.md)). Only nbformat 4 is read.

## What is shown

1. A dim header: kernel name, language version and the number of cells.
2. Every cell, in order, separated by a blank line:
   - Markdown cells go through the [markdown renderer](markdown.md). `attachment:` images are drawn from the cell's attachments.
   - Code cells get a dim `In [n]:` label and syntax highlighting. The language comes from `metadata.kernelspec.language`, then `metadata.language_info.name`, and defaults to python.
   - Raw cells are shown dim.
3. After a code cell with outputs, a dim `Out [n]:` label and the outputs:
   - Streams (stdout and stderr) and `text/plain` results as text.
   - Errors: the traceback, or `name: value` when there is none.
   - `image/png`, `image/jpeg` and `image/svg+xml` drawn inline with kitty, sixel or blocks, at most 20 rows high. Without graphics (ASCII output, `--color=never`, or an image that cannot be decoded) a placeholder such as `[image/png 640x480]` is shown.
   - `text/markdown` rendered as markdown.
   - `text/html` is not converted: `text/plain` is shown when the output has it, else `[html output]`.
   - Other types show as `[mime/type]`.

```
Jupyter notebook · Python 3 3.11.4 · 3 cells

Analysis
━━━━━━━━

In [1]:
  print('hi')
Out [1]:
  hi
```

## Limits and safety

- Notebooks over 64 MiB are not parsed. Image payloads are decoded up to 16 MiB each, and the image decoder has its own limits.
- Each output shows at most 200 lines, then `… N more lines`. A carriage return in a stream (progress bars) keeps only the last part of the line.
- Everything in the file is untrusted. Tracebacks usually carry SGR colours; they follow the `ansi` setting (`keep-sgr` keeps them when colours are on, `escape` shows them as `^[`). Every other escape sequence and control character is shown as `^X`. Code and markdown text are sanitized like in any other view.
- `--plain` hides the header and the `In`/`Out` labels.

## Not a notebook

Invalid JSON, a missing `cells` array or another nbformat version prints `scat: NAME: not a readable notebook (reason); showing JSON` on standard error and shows the file as text. With `renderer = "code"` the JSON source is shown without the note.

## Paging

A notebook that holds image outputs or attachments is never sent to a pager, since graphics escapes must not reach it. Other notebooks page like text. See [paging.md](paging.md).
