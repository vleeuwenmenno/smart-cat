# Terminal detection

scat needs to know what the terminal can do: colours, its background, and which image protocol it speaks. Environment variables give a first guess, but they are wrong over SSH and inside tmux, so scat also asks the terminal.

## The query

scat opens `/dev/tty`, switches it to raw mode and sends one batch:

| Request | Answer used for |
|---|---|
| kitty graphics query (`ESC_Gi=31,...,a=q`) | kitty graphics protocol support |
| `CSI > q` (XTVERSION) | terminal name and version |
| `CSI 16 t` / `CSI 14 t` | cell and window size in pixels |
| `OSC 11 ; ?` | background colour, for picking a light or dark theme |
| `CSI c` (DA1), sent last | sixel support (attribute 4), and the end marker |

Every terminal answers DA1, and answers come back in order. So once the DA1 answer arrives, anything that was not answered is unsupported, and scat does not wait any longer. A 200 ms limit covers terminals that answer nothing. The terminal mode is restored afterwards.

Inside tmux, the kitty query is wrapped in tmux passthrough (needs `set -g allow-passthrough on`).

## When scat does not query

- standard output is not a terminal;
- `TERM` is unset, `dumb`, `linux` or `vt*` (the Linux console and serial lines do not answer);
- nothing being printed needs it. Only syntax highlighting without an explicit theme and images with `--protocol auto|kitty|sixel` trigger the query.

## `scat --diagnose`

Prints everything scat detected:

- config file and how many rules it has, plus any warnings;
- whether stdout is a terminal, colour on/off, colour depth, UTF-8 locale, multiplexer;
- the relevant environment variables (`TERM`, `TERM_PROGRAM`, `COLORTERM`, `KITTY_WINDOW_ID`, ...);
- the query results: terminal version, kitty graphics, sixel, cell size, background, how long it took;
- which image output is used and why (see [images.md](images.md));
- helper tools found on `PATH` (`pdftoppm`, `ffmpeg`, `mpv`, ...).

With file arguments it also shows, per file, the detected type, renderer, syntax, line numbers and matching config rules:

```sh
scat --diagnose src/main.rs README.md
```

Include its output when reporting a rendering bug.
