# Install and update

## Install script

```sh
curl -fsSL https://raw.githubusercontent.com/vleeuwenmenno/smart-cat/main/install.sh | sh
```

The script works from fish, bash and zsh because the pipe runs it with `sh`. It:

1. picks the build for your CPU (`x86_64` or `aarch64`; Linux only);
2. downloads `scat-<target>.tar.gz` and its `.sha256` from GitHub releases;
3. checks the SHA-256 and stops on a mismatch;
4. installs `scat` to `~/.local/bin`, replacing an older version atomically;
5. tells you how to add the directory to `PATH` if it is missing.

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

## Updating

```sh
scat --check-update     # is there a newer release?
scat --update           # install it
scat --update --pre     # include pre-releases
```

Set `channel = "prerelease"` under `[update]` in the [config](configuration.md) to always include pre-releases.

`scat --update` downloads the release for your platform with `curl`, verifies the SHA-256, checks that the new binary runs, and then replaces the running binary in place with a rename. It needs write access to the directory scat lives in. If scat came from a package manager or lives in a system directory, update it the same way you installed it.

## Uninstall

```sh
rm ~/.local/bin/scat
rm -r ~/.config/scat     # optional: your config
```
