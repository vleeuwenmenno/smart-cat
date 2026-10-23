# Using scat as cat

scat can stand in for `cat`. How it behaves depends on the name it is started under: as `scat` it renders on a terminal, as `cat` it is strict and behaves exactly like GNU cat. This page gives the honest picture: what is identical, what changes on purpose, and how to switch each change off.

## Short version

- **Full replacement:** a `cat -> scat` symlink earlier in `PATH`. Started as `cat`, scat is GNU cat, also on a terminal: no rendering, no pager, no directory listing, no terminal query, and messages say `cat:`. Scripts, Makefiles and `PAGER=cat` keep working. You still get scat's extras by asking: `cat -f FILE` renders, and `scat FILE` is the rendering command.
- **Rendering when you type `cat`:** an alias or function in your shell's interactive config. Scripts keep the real cat.
- **Rendering everywhere `cat` is called:** the symlink plus `[cat_mode] render = true`. Not recommended; read the table below first.
- Piped and redirected output is byte-for-byte what GNU cat writes, whatever the setup. The differences only appear when scat renders.

### Installing the symlink

```sh
ln -s "$(command -v scat)" ~/.local/bin/cat   # ~/.local/bin must come before /usr/bin in PATH
```

In fish: `ln -s (command -v scat) ~/.local/bin/cat`. Check with `cat --diagnose | grep invoked`.

## What stays identical

When standard output is not a terminal (`$(cat f)`, `cat a b > c`, `cat f | grep`, `xargs cat`, `find -exec cat`, `watch cat f`, `LESSOPEN="|cat %s"`), scat writes exactly what GNU cat writes, for every cat flag. The test suites `tests/cat_compat.rs` and `tests/script_compat.rs` run the same shell snippets against GNU cat and scat and compare output, exit status and error messages; `tests/script_compat.rs` runs each snippet a second time through a `cat` symlink, where the messages must be identical to GNU cat's, `cat:` prefix included. This covers:

- heredocs, `cat >> file`, `cat -`, `cat - file -`, `cat -- -name`, `/dev/stdin`, `/proc/self/fd/0`, `<(cmd)`, named pipes, `/dev/null`;
- empty files, missing trailing newlines, NUL bytes, invalid UTF-8, CRLF;
- missing files and directories (`cat: x: No such file or directory`, exit 1, continue with the next file), `input file is output file`;
- bad options (exit status 1 and cat's wording), abbreviated long options;
- `set -o pipefail` with `cat big | head -1` (both die by SIGPIPE);
- throughput: the plain copy uses `copy_file_range`, `sendfile` or `splice` like GNU cat.

Remaining differences in piped mode are listed in [cat-compatibility.md](cat-compatibility.md#known-differences-in-piped-output): `--help` and `--version` text differ, and closed stdin or stdout is not reported. Messages say `scat:` when started as `scat`.

## Invoked as `cat`

When the program name (`argv[0]` without its directory) is exactly `cat`:

| | Started as `cat` (default) | With `[cat_mode] render = true` |
|---|---|---|
| Output on a terminal | raw bytes, like GNU cat | rendered, as `scat` |
| Pager | never (also not with `SCAT_PAGING=always`) | as `scat` ([paging.md](paging.md)) |
| `cat dir` | `cat: dir: Is a directory`, exit 1 | listed |
| `cat` with no arguments | reads standard input | lists the directory when stdin is a terminal |
| Terminal query | never | when needed |
| Message prefix | `cat:` | `cat:` |
| scat flags (`-f`, `-p`, `--diagnose`, `--play`, `--update`, ...) | work; `cat -f FILE` renders | work |

`[cat_mode]` has only the one key: once rendering is on, `[defaults]` and the rules apply as for `scat`, so a separate set of keys would only duplicate them. See [configuration.md](configuration.md#cat_mode).

## How each installation method reaches your scripts

| Method | Interactive shell | Scripts | Verdict |
|---|---|---|---|
| `cat -> scat` symlink in `PATH` | strict cat (`cat -f` or `scat` to render) | **yes**, everything: `#!/bin/sh` scripts, Makefiles, `man`, `git`, `systemctl` with `PAGER=cat`; all get GNU cat behaviour | safe; recommended for full replacement |
| symlink plus `[cat_mode] render = true` | renders | **yes**, rendering reaches every program that runs `cat` on your terminal (see the table below) | risky |
| bash `alias cat=scat` in `~/.bashrc` | renders | no: `~/.bashrc` is not read by scripts, and bash does not expand aliases in scripts | safe |
| bash function `cat() { scat "$@"; }` in `~/.bashrc` | renders | no, unless you `export -f cat`: an exported function reaches every bash and `sh` (bash) child script | safe without `export -f` |
| zsh `alias cat=scat` in `~/.zshrc` | renders | no; but anything in `~/.zshenv` is read by every zsh, scripts included | safe in `.zshrc` only |
| fish `alias cat scat` in `config.fish` | renders | **yes**: fish reads `config.fish` and `conf.d/` for scripts and `fish -c` too, and a fish alias is a function | guard with `status is-interactive` |
| fish `functions/cat.fish` | renders | **yes**: autoloaded functions work in every fish, interactive or not | avoid; use the guarded snippet below |

`command cat` and `\cat` (bash, zsh) bypass aliases and functions; in fish, `command cat` does. With a symlink nothing bypasses it except the full path `/usr/bin/cat`.

An alias and the symlink combine well: the symlink makes every `cat` strict, and the alias makes the `cat` you type render.

### fish: alias only in interactive shells

Put this in `~/.config/fish/conf.d/scat.fish` (not in `functions/`, which is autoloaded everywhere):

```fish
if status is-interactive
    function cat --wraps scat --description 'scat in interactive shells'
        scat $argv
    end
end
```

### bash and zsh

```sh
# ~/.bashrc or ~/.zshrc (not ~/.zshenv, not an exported function)
alias cat=scat
```

## What changes when output is a terminal

These apply when scat renders: started as `scat`, through an alias, or as `cat` with `[cat_mode] render = true`. Each row says what can go wrong, how bad it is, and the switch that turns it off. Config keys go in `~/.config/scat/config.toml` (see [configuration.md](configuration.md)); `[defaults]` unless a rule is shown.

| Situation | cat | scat | Rating | Turn it off |
|---|---|---|---|---|
| Any text file | raw bytes | syntax colours, tabs expanded, long lines wrapped | fine | `-p` per run; a rule with `renderer = "text"` or `"raw"` |
| Line numbers | none (unless `-n`) | `   1 │ ` gutter; copying from the terminal copies the numbers | risky for copy-paste | `line_numbers = false`, or `-p` |
| Several files | concatenated | a `── name ──` rule before each, blank line between | fine | `headers = false`, or `-p` |
| Long output | scrolls | at a prompt, goes through `less -RFX`. A script, Makefile or other program running scat **never** gets a pager in `auto` mode: scat only pages when standard input is the terminal too and it leads its own process group, which only a shell with job control (your prompt) arranges. See [paging.md](paging.md#why-the-process-group) | fine | `paging = "never"`, `--paging never`, `SCAT_PAGING=never` or `SCAT_PAGER=` |
| `cat` alone at a prompt | reads the keyboard until Ctrl-D | lists the current directory (`cat > file` and `cat \| cmd` still read the keyboard: their output is not a terminal) | fine | `no_args = "stdin"` |
| `cat some/dir` | `Is a directory`, exit 1 | lists the directory, exit 0 | fine interactively, wrong for scripts that test the status | `directories = "error"` |
| Binary file | raw bytes (often garbage on screen) | hex dump of the first 512 bytes; `cat /bin/ls > copy` is unaffected (piped) | fine | `--raw`, or a rule with `renderer = "raw"` for the type |
| ANSI art, terminal recordings, escape-heavy logs | escapes reach the terminal | colours (SGR) kept, every other escape shown as `^[`, so cursor movement and art break | differs by design (safety) | `ansi = "escape"` is stricter; for art use a rule `match = { ext = ["ans", "nfo"] }` with `renderer = "raw"`, or `--raw` |
| Markdown, CSV, JSON, images, audio, video, compressed files | raw bytes | rendered views, pretty-printed JSON, decompressed text | fine | a rule with `renderer = "raw"` (or `"text"`) |
| `PAGER=cat` / `MANPAGER=cat` / `GIT_PAGER=cat` with a symlink and `render = true` | raw | the program's output is rendered: gutter, and `man`'s overstrikes show as `_^HT` | **breaking** | keep `[cat_mode] render = false` (the default) |
| Terminal query | none | only when something needs it (a highlighted file without a fixed theme, images); answered in a few ms by real terminals, 200 ms timeout on a pty that never answers. The answer is cached per terminal for 10 minutes, so a loop pays once. Keys typed during the query window are lost | fine | a fixed `theme = "..."` (no query for text), `TERM=dumb`, or `renderer = "raw"`; see [terminal-detection.md](terminal-detection.md) |
| `scat file &` (background job) | prints | prints, no pager and no terminal query | fine | |
| `cat /dev/tty`, `cat /dev/urandom \| head`, `tail -f log \| cat` | streams | streams unchanged (devices and piped output are never rendered) | fine | |

## Decisions

The audit of the first version of this page led to three changes:

1. **Strict when invoked as `cat`.** A symlink used to render everywhere, which broke `PAGER=cat` users and made scripts slower. Now the name decides: `cat` is strict unless `[cat_mode] render = true`. This makes the symlink the safe way to replace cat completely.
2. **Page only for a person at the prompt.** A script run from a terminal used to wait in `less` for long output (`less -F` only quits by itself when the output fits). `auto` paging now also needs standard input to be the terminal and scat to lead its own process group. `always` stays an explicit request and pages from scripts too.
3. **Query the terminal only when needed, and cache it.** Plain text, empty files, a fixed theme and renderers that do not use the background never query; answers (including "did not answer") are cached for 10 minutes per terminal session in `$XDG_RUNTIME_DIR/scat/`.

## Strict config

Started as `cat`, scat is already strict. When you run it as `scat` (or through an alias) and want cat behaviour on the terminal except for explicit flags:

```toml
# ~/.config/scat/config.toml
[defaults]
line_numbers = false
paging = "never"
no_args = "stdin"
directories = "error"
headers = false

# Every file is copied byte for byte, exactly like cat, also to a terminal.
[[rule]]
match = { mime = "*" }
renderer = "raw"
```

A lighter variant keeps highlighting but drops everything that can surprise a habit: the `[defaults]` block above without the rule, plus a fixed `theme = "Monokai Extended"` so there is no terminal query.

## One-off escapes

| Want | Use |
|---|---|
| real cat, once | `command cat f`, `\cat f` (bash/zsh), `/usr/bin/cat f` |
| rendering through a `cat` symlink | `cat -f f`, or `scat f` |
| scat without rendering | `scat --raw f` |
| no gutter or headers | `scat -p f` |
| no pager | `scat --paging never f` |
| rendering into a pipe | `scat -f f \| less -R` |
