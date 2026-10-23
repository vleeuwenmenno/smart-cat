# Syntax highlighting

scat uses [syntect](https://github.com/trishume/syntect) with the syntax and theme set that [bat](https://github.com/sharkdp/bat) ships (through [two-face](https://github.com/CosmicHorrorDev/two-face)), so languages and themes match bat's.

## Language detection

In order:

1. `-l` / `--language NAME`, or `language = "NAME"` in a matching config rule. NAME can be a language name (`Rust`) or an extension (`rs`).
2. The file name: full names like `Makefile` or `.bashrc` first, then the extension (`main.rs`).
3. The first line: shebangs (`#!/usr/bin/env python3`), `<?xml`, modelines.

If nothing matches, the file is printed as plain text. Standard input has no file name, so only the first line or `-l` can identify it:

```sh
curl -s https://example.com/app.js | scat -l js
```

`scat --list-languages` lists every language with its extensions. `scat --diagnose FILE` shows which syntax scat picked for a file.

## Themes

`scat --list-themes` lists the themes. Choose one with `--theme NAME`, `SCAT_THEME`, `theme = "NAME"` in `[defaults]`, or per file in a rule.

Without an explicit theme scat picks:

| Situation | Theme |
|---|---|
| 16-colour terminal (`TERM=linux`, Linux console) | `ansi`, which uses the terminal's own palette |
| light terminal background | `theme_light` (default `Monokai Extended Light`) |
| otherwise | `theme_dark` (default `Monokai Extended`) |

The background colour comes from the [terminal query](terminal-detection.md) (OSC 11), or from `COLORFGBG` when the terminal does not answer. Setting a fixed `theme` skips the query entirely, which saves up to 200 ms in environments that never answer (a bare pty under `script`, some IDE terminals).

## Colour depth

| Depth | Detected from | Output |
|---|---|---|
| 24-bit | `COLORTERM=truecolor` or `24bit`, `TERM=*-direct` | exact theme colours |
| 256 colours | `TERM` containing `256color`, kitty, foot, ghostty | nearest xterm-256 colour |
| 16 colours | `TERM=linux`, `vt*`, anything else | nearest ANSI colour |

Colours are off when `NO_COLOR` is set, `TERM=dumb`, `--color=never`, or `CLICOLOR=0`. `--color=always` or `CLICOLOR_FORCE=1` turns them on even in a pipe (together with `-f`).

## Speed

Syntaxes and themes load only when a file needs highlighting. Highlighting uses the Oniguruma regex engine (as bat does), which is about four times faster than the pure-Rust engine for syntaxes like Rust.
