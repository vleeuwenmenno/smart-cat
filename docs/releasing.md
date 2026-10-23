# Releasing

Releases are built by GitHub Actions (`.github/workflows/release.yml`) when a version tag is pushed.

## Stable release

```sh
make bump VERSION=0.2.0
git push --follow-tags
```

`make bump` requires a clean working tree. It sets `version` in `Cargo.toml`, refreshes `Cargo.lock`, commits `chore: release v0.2.0` and creates the annotated tag `v0.2.0`.

## Pre-release

Any version with a pre-release suffix becomes a GitHub pre-release:

```sh
make bump VERSION=0.2.0-rc.1
git push --follow-tags
```

Pre-releases are never marked "latest", so the default install script and `scat --update` skip them. Users get them with `--pre` or `channel = "prerelease"`.

## What the workflow does

1. Checks that the tag matches the `Cargo.toml` version.
2. On native runners (`ubuntu-24.04` for x86_64, `ubuntu-24.04-arm` for aarch64), runs the tests and builds a static musl binary with `make dist DIST_CC=musl-gcc`.
3. Smoke-tests the archive.
4. Creates the GitHub release with generated notes and uploads:
   - `scat-x86_64-unknown-linux-musl.tar.gz` and `.sha256`
   - `scat-aarch64-unknown-linux-musl.tar.gz` and `.sha256`

Asset names carry no version on purpose: `install.sh` uses GitHub's `releases/latest/download/<asset>` URL, and `scat --update` looks assets up by name.

## Building a release archive locally

```sh
make dist                                      # for this machine's CPU
make dist TARGET=aarch64-unknown-linux-musl DIST_CC=aarch64-linux-musl-gcc
```

The musl target needs a C compiler for Oniguruma. `make dist` uses `musl-gcc` when installed (Arch: `pacman -S musl`, Debian/Ubuntu: `apt install musl-tools`). Without it, and only when building for the host's own CPU, it falls back to `gcc`, which also produces a working static binary. Cross builds need `DIST_CC` set to a cross compiler.

## CI

`.github/workflows/ci.yml` runs `make check` (rustfmt, clippy with warnings as errors, tests) and `make shellcheck` on every push to `main` and on pull requests.
