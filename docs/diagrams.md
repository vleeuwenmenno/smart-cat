# Diagrams

Rendering applies in a terminal only. When output is piped or redirected, a diagram source is printed byte for byte like `cat`.

`scat flow.mmd` draws a mermaid, graphviz or PlantUML diagram as an image, with a dim caption that names the file, the diagram type and what drew it:

```
<the diagram, drawn with kitty, sixel or Unicode blocks>
flow.mmd  mermaid diagram (built in)
```

| Type | Extensions | Drawn by |
|---|---|---|
| mermaid | `.mmd`, `.mermaid` | the built-in renderer ([`mermaid-rs-renderer`](https://crates.io/crates/mermaid-rs-renderer)); when it fails, `mmdc` (mermaid-cli) if it is on `PATH` |
| graphviz | `.dot`, `.gv` | `dot -Tsvg` if graphviz is installed; otherwise the built-in [`layout-rs`](https://crates.io/crates/layout-rs), which handles a subset of dot |
| PlantUML | `.puml`, `.plantuml`, `.pu` | `plantuml -tsvg -pipe` only; there is no built-in renderer |

`-l mermaid`, `-l dot` (or `graphviz`) and `-l plantuml` pick the type for a file with another name, as does a [rule](configuration.md) with `language = "mermaid"`.

Every renderer produces SVG, which is drawn like an [SVG image](images.md): the same output methods, size limits and fonts.

Typical timings (release build, kitty, whole run): a small mermaid flowchart about 25 ms (the first run after boot up to 90 ms while fonts are read), a small dot graph about 35 ms with `dot` and 20 ms with the built-in layout.

## Built-in mermaid support

The built-in renderer covers flowcharts (`flowchart`, `graph`), sequence, class, state and ER diagrams, pie, XY and quadrant charts, gantt, timeline, journey, mindmap, git graphs and more (`mermaid-rs-renderer` 0.3 lists 23 diagram types). It needs no browser or Node.js and takes a few milliseconds. Syntax it does not understand falls back to `mmdc` when installed, else to the source.

## When the source is shown instead

The source is shown, highlighted like any text file, when:

- images cannot be drawn: ASCII output (colours off, or no UTF-8 locale), or `-f` into a pipe without `--protocol`;
- drawing fails: a syntax error, a diagram type no installed renderer knows, or a renderer that is missing (PlantUML without `plantuml`). A short note goes to stderr first, for example
  `scat: g.dot: cannot draw graphviz diagram: dot: <stdin>: syntax error in line 1 near ';'; showing the source`
  or `scat: seq.puml: cannot draw PlantUML diagram: no renderer available; install plantuml to draw it; showing the source`. The exit status stays 0 and scat goes on with the next file;
- the file is larger than 1 MiB;
- a rule says `renderer = "code"` (or `"text"`).

`renderer = "diagram"` forces the view for any file. Without a diagram extension or language the type is guessed from the first line (`digraph`, `graph {`, `@startuml`, mermaid keywords such as `flowchart` or `sequenceDiagram`).

## Diagrams in markdown

Fenced ` ```mermaid ` and ` ```dot ` / ` ```graphviz ` blocks in a [markdown](markdown.md) document are drawn in place, like local images: at most 20 rows high and as wide as the text column, with a dim `mermaid diagram` caption below. A block stays highlighted code when images cannot be drawn (ASCII output, colours off) or rendering fails (with a note on stderr). PlantUML blocks are not drawn: each would start a JVM.

A markdown file with such a block is not sent through the [pager](paging.md), like one with images, since image escapes must never reach a pager. Diagram files themselves are never paged either.

## Safety

- Diagram sources are untrusted. The built-in renderers run in a child process (scat itself with a hidden `--internal-diagram` argument), with a 2 GiB address space limit, a CPU time limit, no core dumps and a 10 second timeout. A panic or a hang in a renderer becomes a note; scat goes on.
- External tools run with an argv array (never through a shell), the source on standard input, a timeout (10 seconds for `dot`, 30 seconds for `plantuml` and `mmdc`, which start a JVM or a browser) and a 32 MiB cap on the SVG they return.
- **No network access.** The PlantUML web server is never used. `plantuml` runs with `PLANTUML_SECURITY_PROFILE=SANDBOX`, which blocks `!include` of files and URLs. The SVG is drawn with external references disabled, as for any SVG.
- Error messages from renderers are sanitized before they are printed.

## Tools

`scat --diagnose` lists `dot`, `plantuml` and `mmdc`, and for a diagram file the type and the renderers it would try:

```
  g.dot
    renderer: diagram (auto)
    diagram: graphviz, drawn by dot, then built in
```

Install graphviz (`pacman -S graphviz`, `apt install graphviz`) for full dot support, `plantuml` for PlantUML, and `@mermaid-js/mermaid-cli` (`mmdc`) for mermaid syntax the built-in renderer does not cover. `mmdc` runs with HTML labels turned off, since the SVG renderer cannot draw HTML.
