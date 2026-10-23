# Paging

Long text output goes through a pager, so a big file does not scroll past. Paging is on by default (`auto`) and you never notice it for short output.

## When scat pages

All of these must hold:

- standard output is a terminal, and scat is rendering (not `--raw`, not a cat flag like `-A`, not piped);
- every file is text-like: source, markdown, plain text, directory listings (except tiles with kitty or sixel thumbnails), or a [hex dump](rendering.md#hex-dump). Any image, audio, video or PDF in the list, a markdown file with images (`![...]`), a [font](fonts.md) whose specimen will be drawn, or a compressed file turns paging off for the whole run, because a pager cannot show graphics and `--play` hands the terminal to mpv;
- no file comes from standard input (scat cannot look at a stream ahead of time);
- scat is in the terminal's foreground process group. A background job (`scat log &`) prints directly, as cat does; a pager there would be stopped by the shell until you bring it to the foreground;
- scat was not started under the name `cat` (a `cat -> scat` symlink is strict, see [using-as-cat.md](using-as-cat.md)), unless `[cat_mode] render = true` is set.

In `auto` mode two more conditions must hold, so that only a person at the prompt gets a pager:

- standard input is a terminal too;
- scat leads its own process group.

### Why the process group

The goal: `cat bigfile` inside a script run from a terminal must never wait in `less`. Standard input and output are both the terminal there, and the script is in the foreground, so those checks do not tell a script apart from a prompt.

Process groups do. A shell with job control (every interactive bash, zsh and fish) starts each command line you type as a new process group, with the first command as its leader. A script runs without job control, so the commands it starts stay in the script's own group and are never its leader. The same holds for anything else that starts scat as a child: `make`, `xargs`, `find -exec`, `git`, an editor, Python's `subprocess`. scat checks `getpgrp() == getpid()`, one system call and no guessing.

Other heuristics were rejected. The parent's command line in `/proc/<ppid>/cmdline` (is it a shell, with or without a script argument?) breaks as soon as something sits between the shell and scat: `sudo`, a wrapper script, an editor's terminal, `script`; and every shell spells "interactive" differently. `$PS1` is not exported, so scat never sees it.

What this means in practice:

| Started as | Pages long output (`auto`) |
|---|---|
| `scat big.log` at a prompt (also through an alias or a shell function) | yes |
| `exec scat big.log`, `ssh -t host scat big.log` | yes (scat replaces the group leader) |
| a line in `./script.sh`, `bash -c 'scat a; scat b'`, a Makefile, `xargs`, `find -exec` | no, prints like cat |
| `sudo scat big.log` | depends on sudo: with `use_pty` (the default since sudo 1.9.14) the command leads its own group and pages; without it, it prints |
| `scat big.log < input` | no: standard input is not the terminal |
| `scat big.log &` | no: background job |

`--diagnose` shows the three facts and whether `auto` would page.

### `always`

`--paging always` (or `paging = "always"`, `SCAT_PAGING=always`) is an explicit request, so it skips the standard input and process group checks: less reads its keys from `/dev/tty`, not from standard input, and works fine from a script. It still needs standard output to be the terminal, scat to be in the foreground, and text-like files. Note that `paging = "always"` in the config therefore also pages inside scripts.

scat decides from the file list before it prints anything, by sniffing the first 8 KiB of each file. In `auto` mode the output is held back until it would fill the screen. Short output is written directly and no pager starts. That is why `auto` is the default: nothing changes for `scat small.txt`, and the pager only appears when you need it. With `always`, the pager starts for any output.

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
