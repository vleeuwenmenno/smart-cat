# Paging

Long text output goes through a pager, so a big file does not scroll past. Paging is on by default (`auto`) and you never notice it for short output.

## When scat pages

All of these must hold:

- standard output is a terminal, and scat is rendering (not `--raw`, not a cat flag like `-A`, not piped);
- every file is text-like: source, markdown, plain text, directory listings, or a [hex dump](rendering.md#hex-dump). Any image, audio, video or PDF in the list, a markdown file with images (`![...]`), or a compressed file turns paging off for the whole run, because a pager cannot show graphics and `--play` hands the terminal to mpv;
- no file comes from standard input (scat cannot look at a stream ahead of time).
- scat is in the terminal's foreground process group. A background job (`scat log &`) prints directly, as cat does; a pager there would be stopped by the shell until you bring it to the foreground.

Scat decides from the file list before it prints anything, by sniffing the first 8 KiB of each file. In `auto` mode the output is held back until it would fill the screen. Short output is written directly and no pager starts. That is why `auto` is the default: nothing changes for `scat small.txt`, and the pager only appears when you need it. With `always`, the pager starts for any output.

## Settings

| Setting | Values |
|---|---|
| `--paging WHEN` or `SCAT_PAGING` | `auto`, `always`, `never` |
| `[defaults] paging` | same; default `"auto"` |
| `SCAT_PAGER` | pager command; an empty value turns paging off |
| `PAGER` | pager command when `SCAT_PAGER` is unset |

The default pager is `less -RFX`: `-R` passes colours, `-F` quits at once when the output fits one screen, `-X` leaves the text on screen after you quit. A bare `less` in `SCAT_PAGER` or `PAGER` gets `-RFX` too. `PAGER=cat` (or `scat`) means no paging.

The command is split into words like a shell would (single and double quotes, backslash), but never run through a shell: no `$VAR` expansion, no `;`, no pipes. The program is looked up in the absolute entries of `PATH`, or must be an absolute path; it is never taken from the current directory. If it cannot be found, scat prints without a pager (and warns when you set `SCAT_PAGER` or used `--paging always`).

## Quitting early

When you quit the pager before the output is done, scat stops quietly and exits with status 0. When scat finishes first, it waits for you to quit the pager.

## Headers

When several files are rendered to a terminal, each gets a dim rule with its name, and a blank line separates files:

```
── src/main.rs ─────────────────────────
```

`-p` / `--plain` hides the rules for one run. `[defaults] headers = false` hides them always. A single file, piped output and directory listings (which have their own headings) get none.
