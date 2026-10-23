# Using scat as cat

scat can stand in for `cat`, but how safe that is depends on how you install it. This page gives the honest picture: what is identical, what changes on purpose, and how to switch each change off.

## Short version

- **Recommended:** make `cat` call scat only in interactive shells (an alias or function in your shell's interactive config). Scripts keep the real cat.
- **Not recommended:** a `cat -> scat` symlink earlier in `PATH`. It reaches every script, Makefile and program, including ones that run `$PAGER` (set to `cat`) on your terminal. If you do it anyway, use the [strict config](#strict-config).
- Piped and redirected output is byte-for-byte what GNU cat writes, whatever the setup. The differences only appear when scat's standard output is a terminal.

## What stays identical

When standard output is not a terminal (`$(cat f)`, `cat a b > c`, `cat f | grep`, `xargs cat`, `find -exec cat`, `watch cat f`, `LESSOPEN="|cat %s"`), scat writes exactly what GNU cat writes, for every cat flag. The test suites `tests/cat_compat.rs` and `tests/script_compat.rs` run the same shell snippets against GNU cat and scat and compare output, exit status and error messages. This covers:

- heredocs, `cat >> file`, `cat -`, `cat - file -`, `cat -- -name`, `/dev/stdin`, `/proc/self/fd/0`, `<(cmd)`, named pipes, `/dev/null`;
- empty files, missing trailing newlines, NUL bytes, invalid UTF-8, CRLF;
- missing files and directories (`cat: x: No such file or directory`, exit 1, continue with the next file), `input file is output file`;
- bad options (exit status 1 and cat's wording), abbreviated long options;
- `set -o pipefail` with `cat big | head -1` (both die by SIGPIPE);
- throughput: the plain copy uses `copy_file_range`, `sendfile` or `splice` like GNU cat.

Remaining differences in piped mode are listed in [cat-compatibility.md](cat-compatibility.md#known-differences-in-piped-output): messages say `scat:` instead of `cat:` (also through a symlink named `cat`), `--help` and `--version` text differ, and closed stdin or stdout is not reported.

## How each installation method reaches your scripts

| Method | Interactive shell | Scripts | Verdict |
|---|---|---|---|
| bash `alias cat=scat` in `~/.bashrc` | yes | no: `~/.bashrc` is not read by scripts, and bash does not expand aliases in scripts | safe |
| bash function `cat() { scat "$@"; }` in `~/.bashrc` | yes | no, unless you `export -f cat`: an exported function reaches every bash and `sh` (bash) child script | safe without `export -f` |
| zsh `alias cat=scat` in `~/.zshrc` | yes | no; but anything in `~/.zshenv` is read by every zsh, scripts included | safe in `.zshrc` only |
| fish `alias cat scat` in `config.fish` | yes | **yes**: fish reads `config.fish` and `conf.d/` for scripts and `fish -c` too, and a fish alias is a function | guard with `status is-interactive` |
| fish `functions/cat.fish` | yes | **yes**: autoloaded functions work in every fish, interactive or not | avoid; use the guarded snippet below |
| `cat -> scat` symlink in `PATH` | yes | **yes**, everything: `#!/bin/sh` scripts, Makefiles, `man`, `git`, `systemctl` with `PAGER=cat` | risky; strict config only |

`command cat` and `\cat` (bash, zsh) bypass aliases and functions; in fish, `command cat` does. With a symlink nothing bypasses it except the full path `/usr/bin/cat`.

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

These are by design. Each row says what can go wrong, how bad it is, and the switch that turns it off. Config keys go in `~/.config/scat/config.toml` (see [configuration.md](configuration.md)); `[defaults]` unless a rule is shown.

| Situation | cat | scat | Rating | Turn it off |
|---|---|---|---|---|
| Any text file | raw bytes | syntax colours, tabs expanded, long lines wrapped | fine | `-p` per run; a rule with `renderer = "text"` or `"raw"` |
| Line numbers | none (unless `-n`) | `   1 │ ` gutter; copying from the terminal copies the numbers | risky for copy-paste | `line_numbers = false`, or `-p` |
| Several files | concatenated | a `── name ──` rule before each, blank line between | fine | `headers = false`, or `-p` |
| Long output | scrolls | goes through `less -RFX`; a script run from a terminal **waits for `q`** when the output is longer than the screen (`less -F` only quits by itself for short output) | risky in scripts | `paging = "never"`, `--paging never`, `SCAT_PAGING=never` or `SCAT_PAGER=` |
| `cat` alone at a prompt | reads the keyboard until Ctrl-D | lists the current directory (`cat > file` and `cat \| cmd` still read the keyboard: their output is not a terminal) | fine | `no_args = "stdin"` |
| `cat some/dir` | `Is a directory`, exit 1 | lists the directory, exit 0 | fine interactively, wrong for scripts that test the status | `directories = "error"` |
| Binary file | raw bytes (often garbage on screen) | hex dump of the first 512 bytes; `cat /bin/ls > copy` is unaffected (piped) | fine | `--raw`, or a rule with `renderer = "raw"` for the type |
| ANSI art, terminal recordings, escape-heavy logs | escapes reach the terminal | colours (SGR) kept, every other escape shown as `^[`, so cursor movement and art break | differs by design (safety) | `ansi = "escape"` is stricter; for art use a rule `match = { ext = ["ans", "nfo"] }` with `renderer = "raw"`, or `--raw` |
| Markdown, CSV, JSON, images, audio, video, compressed files | raw bytes | rendered views, pretty-printed JSON, decompressed text | fine | a rule with `renderer = "raw"` (or `"text"`) |
| `PAGER=cat` / `MANPAGER=cat` / `GIT_PAGER=cat` with a symlink | raw | the program's output is rendered: gutter, and `man`'s overstrikes show as `_^HT` | **breaking** with a symlink | strict config, or do not use a symlink |
| Terminal query | none | the first highlighted file sends a query to `/dev/tty` (background colour, graphics). Answered in a few ms by real terminals; 200 ms timeout on a pty that never answers (some CI and `expect` setups); keys typed during that window are lost | risky in loops on odd terminals | set a fixed `theme = "..."` (no query for text), `TERM=dumb`, or `renderer = "raw"` |
| `scat file &` (background job) | prints | prints, no pager and no terminal query | fine | |
| `cat /dev/tty`, `cat /dev/urandom \| head`, `tail -f log \| cat` | streams | streams unchanged (devices and piped output are never rendered) | fine | |

## Strict config

Everything above off: on a terminal scat then behaves like cat except for the explicit `scat` flags. Use this when scat is installed as `cat` through a symlink.

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

A lighter variant keeps highlighting but drops everything that can surprise a script or a habit: the `[defaults]` block above without the rule, plus a fixed `theme = "Monokai Extended"` so there is no terminal query.

## One-off escapes

| Want | Use |
|---|---|
| real cat, once | `command cat f`, `\cat f` (bash/zsh), `/usr/bin/cat f` |
| scat without rendering | `scat --raw f` |
| no gutter or headers | `scat -p f` |
| no pager | `scat --paging never f` |
| rendering into a pipe | `scat -f f \| less -R` |
