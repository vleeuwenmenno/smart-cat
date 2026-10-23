# Install and update

## Install script

```sh
curl -fsSL https://raw.githubusercontent.com/vleeuwenmenno/smart-cat/main/install.sh | sh
```

The script works from fish, bash and zsh because the pipe runs it with `sh`. It:

1. picks the build for your CPU (`x86_64` or `aarch64`; Linux only);
2. downloads `scat-<target>.tar.gz` and its `.sha256` from GitHub releases;
3. checks the SHA-256 and stops on a mismatch;
4. copies the binary to a fresh `mktemp` file next to the target (`~/.local/bin/.scat.XXXXXX`), runs its `--version`, and only then renames it over `scat`, so a broken download keeps the old install; the staged file is removed on any failure;
5. tells you how to add the directory to `PATH` if it is missing.

The script body is one function called on its last line, so a download cut short by `curl | sh` runs nothing.

Options go after `sh -s --`:

```sh
curl -fsSL https://raw.githubusercontent.com/vleeuwenmenno/smart-cat/main/install.sh | sh -s -- --pre
curl -fsSL https://raw.githubusercontent.com/vleeuwenmenno/smart-cat/main/install.sh | sh -s -- --version v0.1.0
curl -fsSL https://raw.githubusercontent.com/vleeuwenmenno/smart-cat/main/install.sh | sh -s -- --dir /usr/local/bin
```

| Option | Effect |
|---|---|
| `--pre` | newest release including pre-releases |
| `--version TAG` | a specific release |
| `--dir DIR` | install directory (also `SCAT_INSTALL_DIR`) |

Running the script again updates scat.

The binaries are static (musl), so they run on any Linux distribution without extra libraries.

## From source

```sh
git clone https://github.com/vleeuwenmenno/smart-cat
cd smart-cat
make install        # builds and installs to ~/.local/bin/scat
```

`make install` builds for your machine with the normal toolchain; it does not need musl. Use `make install BINDIR=/some/dir` for another location, and `make uninstall` to remove it. Building needs a C compiler (for the Oniguruma regex library). `make` alone lists every target.

## When the installer fails

| Message | Cause | Fix |
|---|---|---|
| `repository is private or does not exist` | the GitHub repository is not public | install from source |
| `no stable release yet; the newest is pre-release vX` | only pre-releases exist | add `--pre` |
| `no release asset at ...` | the release has no build for your CPU, or the tag does not exist | check the tag, or install from source |

## Completions and man page

`install.sh` and `make install` set these up for you. They run `scat --install-completions`, which writes:

| For | File |
|---|---|
| fish (if installed) | `~/.config/fish/completions/scat.fish` |
| bash (if installed) | `~/.local/share/bash-completion/completions/scat` |
| zsh (if installed) | `~/.local/share/zsh/site-functions/_scat` |
| `man scat` | `~/.local/share/man/man1/scat.1` |

`XDG_CONFIG_HOME` and `XDG_DATA_HOME` are honoured. fish and bash pick the files up in a new shell. zsh needs that directory on `fpath`; scat prints the line to add to `~/.zshrc` when it writes the zsh file.

- `scat --update` rewrites the files that are already installed, so completions always match the new flags.
- `scat --uninstall-completions` removes them; `make uninstall` does this too.
- `install.sh --no-completions` skips them.
- To manage them yourself, print them with `scat --completions fish|bash|zsh|elvish|powershell` and `scat --man`.

## Updating

```sh
scat --check-update     # is there a newer release?
scat --update           # install it
scat --update --pre     # include pre-releases
```

Set `channel = "prerelease"` under `[update]` in the [config](configuration.md) to always include pre-releases.

`scat --update` looks through all releases (up to 1000), downloads the archive for your platform with `curl`, and verifies its SHA-256 in memory. It stages the archive in a new private directory (mode 0700, random name) next to the binary, extracts `scat` there, and checks that it is a regular file (not a symlink or directory) whose `--version` prints exactly `scat <new version>`. Only then does it replace the running binary with a rename. It needs write access to the directory scat lives in. If scat came from a package manager or lives in a system directory, update it the same way you installed it.

A scat built by cargo (running from a `target/debug` or `target/release` directory) refuses to update itself, since `make` would overwrite it again; rebuild from source instead. Set `SCAT_UPDATE_FORCE=1` to replace it anyway.

## Uninstall

```sh
rm ~/.local/bin/scat
rm -r ~/.config/scat     # optional: your config
```
