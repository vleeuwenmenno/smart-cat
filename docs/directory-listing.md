# Directory listing

When scat renders (standard output is a terminal, or `--force-render`), a directory is listed instead of failing.

```sh
scat              # no arguments, in a terminal: list the current directory
scat src docs     # several directories: a "dir:" header before each
scat --all .      # include hidden (dot) entries
scat --long .     # one entry per line: type, size, age, name
```

## When it applies

| Situation | Behaviour |
|---|---|
| `scat`, stdin and stdout are terminals | list the current directory |
| `echo hi \| scat` (stdin not a terminal) | read stdin, like cat |
| `scat` with stdout piped | read stdin, like cat |
| `scat some/dir` when rendering | list it |
| `scat some/dir \| x` | `scat: some/dir: Is a directory`, exit 1, like cat |

Files and directories can be mixed; each is handled in argument order. An unreadable directory prints `scat: NAME: Permission denied`, sets exit status 1 and scat continues. Opt out in the [config](configuration.md): `no_args = "stdin"` disables the no-argument listing, `directories = "error"` makes directory arguments an error again.

## Layout

- Directories first, then files; each group sorted case-insensitively with numbers compared by value (`file2` before `file10`).
- Hidden entries are not shown unless `--all` or `[list] show_hidden = true`.
- Grid like `ls`: columns fill top to bottom and use the terminal width (the tty size, else `$COLUMNS`, else 80). Column widths use display width, so wide characters stay aligned.
- Directories get a trailing `/`. Symlinks are only coloured in the grid; `--long` shows `name -> target`.
- `--long` (or `[list] style = "long"`): type letter, size (`1.5 KiB`, `-` for directories), age (`3 d ago`), name.

## Colours

When colours are on, `LS_COLORS` is honoured: the keys `di`, `ln`, `or`, `ex`, `fi`, `so`, `pi`, `bd`, `cd` and `*.ext` patterns. Values may only contain digits and `;`. Without `LS_COLORS`: directories bold blue, symlinks cyan, executables bold green, hidden entries dim.

## Safety

File names are untrusted. Control characters are shown as `^X` (a newline in a name is `^J`), and invisible formatting characters such as bidi overrides are replaced by `U+FFFD`, so a name cannot emit escape sequences or break the grid.
