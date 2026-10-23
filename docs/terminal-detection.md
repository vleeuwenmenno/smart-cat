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

While the terminal is in raw mode, scat blocks `SIGINT`, `SIGTERM`, `SIGHUP`, `SIGQUIT` and `SIGTSTP`. A Ctrl-C (or `kill`) during the query is delivered right after the terminal mode is restored, so it can never leave the terminal raw.

### Typeahead

Answers and keystrokes arrive on the same `/dev/tty`. If input is already waiting when scat is about to query (you typed ahead while a previous command ran), scat skips the query and uses the environment hints instead, so that input is left for your shell. Keys typed during the query window itself (at most 200 ms, usually a few ms) are still read along with the answers and lost; that small window is accepted.

Inside tmux, the kitty query is wrapped in tmux passthrough (needs `set -g allow-passthrough on`).

## When scat does not query

- standard output is not a terminal;
- `TERM` is unset, `dumb`, `linux` or `vt*` (the Linux console and serial lines do not answer);
- scat runs as a background job (`scat file &`): changing the terminal mode would stop it with `SIGTTOU`;
- scat was started under the name `cat` and is strict (see [using-as-cat.md](using-as-cat.md));
- a fresh [cache](#cache) entry exists;
- nothing being printed needs it. The query is made on first use, by what needs it:

| Needs the query | Does not |
|---|---|
| a file with a known syntax, highlighted without a fixed theme (to pick `theme_dark` or `theme_light`) | plain text with no syntax found, empty files, files over `max_highlight_bytes` |
| markdown code blocks, without a fixed theme | colours off (`--color never`, `NO_COLOR`, piped), `--theme`, `SCAT_THEME`, `theme` in `[defaults]` or a rule |
| markdown inline code, without a fixed theme (with one, the theme's own background decides) | 16-colour terminals (the `ansi` theme) |
| images, PDF pages, audio cover art and video frames with `--protocol auto\|kitty\|sixel` | diffs, logs, tables, hex dumps, directory listings, `--raw`, cat flags |
| `--play` with video (picks mpv's output) | `--play` with audio |

## Cache

So that a series of commands in one terminal does not pay for the query every time (a few ms on a real terminal, the full 200 ms on a pty that never answers), the result is kept for 10 minutes in `$XDG_RUNTIME_DIR/scat/term-<key>`.

- **Key:** a hash of the controlling terminal's device number, the session id (`getsid`), `TERM`, `TERM_PROGRAM`, `KITTY_WINDOW_ID` and `TMUX_PANE`. A new tab, window, login, ssh session or tmux pane gets its own entry.
- **Stored:** kitty graphics support, sixel support, cell size in pixels, background colour, and whether the terminal answered at all. "Did not answer" is cached too, so a silent pty costs 200 ms once, not on every run.
- **Not stored:** a query that could not run (background job, typeahead) is not cached.
- **Lifetime:** 10 minutes. The background colour changes when you switch the terminal's theme; scat picks that up once the entry expires. Delete `$XDG_RUNTIME_DIR/scat/` to pick it up at once, or run `scat --diagnose`, which always queries and refreshes the entry.
- **Safety:** the directory is created with mode 0700 and files with 0600. An entry that is not owned by you, is readable by others, is a symlink or does not parse is ignored.
- **Off:** without `XDG_RUNTIME_DIR` (or without a controlling terminal) there is no cache and every run that needs the answers queries.

## `scat --diagnose`

Prints everything scat detected:

- config file and how many rules it has, plus any warnings, and whether scat was invoked as `cat` (strict or rendering);
- the paging facts: standard input a terminal, foreground, own process group, and whether `auto` would page (see [paging.md](paging.md#why-the-process-group));
- whether stdout is a terminal, colour on/off, colour depth, UTF-8 locale, multiplexer;
- the relevant environment variables (`TERM`, `TERM_PROGRAM`, `COLORTERM`, `KITTY_WINDOW_ID`, ...);
- whether a fresh cache entry existed (the query itself always runs fresh and refreshes the entry);
- the query results: terminal version, kitty graphics, sixel, cell size, background, how long it took;
- which image output is used and why (see [images.md](images.md));
- helper tools found on `PATH` (`pdftoppm`, `ffmpeg`, `mpv`, ...).

With file arguments it also shows, per file, the detected type, renderer, syntax, line numbers and matching config rules:

```sh
scat --diagnose src/main.rs README.md
```

Include its output when reporting a rendering bug.
