#!/bin/sh
# Install or update scat from GitHub releases.
#
#   curl -fsSL https://raw.githubusercontent.com/vleeuwenmenno/smart-cat/main/install.sh | sh
#   curl -fsSL https://raw.githubusercontent.com/vleeuwenmenno/smart-cat/main/install.sh | sh -s -- --pre
#
# Options:
#   --pre             install the newest release, including pre-releases
#   --version TAG     install a specific release, e.g. v0.2.0
#   --dir DIR         install into DIR (default: $SCAT_INSTALL_DIR or ~/.local/bin)
#   --help            show this help
set -eu

REPO="${SCAT_REPO:-vleeuwenmenno/smart-cat}"
INSTALL_DIR="${SCAT_INSTALL_DIR:-$HOME/.local/bin}"
PRE=0
VERSION=""
TMP=""
STAGED=""

say() { printf '%s\n' "$*"; }
die() { printf 'scat install: %s\n' "$*" >&2; exit 1; }

# Removes the download directory and a staged binary that was not installed.
cleanup() {
    if [ -n "$TMP" ]; then rm -rf "$TMP"; fi
    if [ -n "$STAGED" ]; then rm -f "$STAGED"; fi
}

usage() {
    say "usage: install.sh [--pre] [--version TAG] [--dir DIR]"
    say ""
    say "  --pre          install the newest release, including pre-releases"
    say "  --version TAG  install a specific release, e.g. v0.2.0"
    say "  --dir DIR      install into DIR (default: \$SCAT_INSTALL_DIR or ~/.local/bin)"
}

# The whole script is a function called on the last line, so a download cut short
# by `curl | sh` never runs half of it.
main() {
    while [ $# -gt 0 ]; do
        case "$1" in
            --pre) PRE=1 ;;
            --version)
                [ $# -ge 2 ] || die "--version needs a tag"
                VERSION="$2"
                shift
                ;;
            --dir)
                [ $# -ge 2 ] || die "--dir needs a directory"
                INSTALL_DIR="$2"
                shift
                ;;
            -h | --help)
                usage
                exit 0
                ;;
            *) die "unknown option: $1" ;;
        esac
        shift
    done

    [ "$(uname -s)" = "Linux" ] || die "only Linux builds are published (found $(uname -s))"
    case "$(uname -m)" in
        x86_64 | amd64) TARGET="x86_64-unknown-linux-musl" ;;
        aarch64 | arm64) TARGET="aarch64-unknown-linux-musl" ;;
        *) die "no build for $(uname -m); build from source with: cargo install --git https://github.com/$REPO" ;;
    esac

    if command -v curl >/dev/null 2>&1; then
        fetch() { curl -fsSL --proto '=https' --tlsv1.2 "$1"; }
        download() { curl -fsSL --proto '=https' --tlsv1.2 -o "$2" "$1"; }
    elif command -v wget >/dev/null 2>&1; then
        fetch() { wget -qO- "$1"; }
        download() { wget -qO "$2" "$1"; }
    else
        die "curl or wget is required"
    fi
    command -v tar >/dev/null 2>&1 || die "tar is required"
    if command -v sha256sum >/dev/null 2>&1; then
        sha256() { sha256sum "$1" | cut -d' ' -f1; }
    elif command -v shasum >/dev/null 2>&1; then
        sha256() { shasum -a 256 "$1" | cut -d' ' -f1; }
    else
        die "sha256sum or shasum is required"
    fi

    ASSET="scat-$TARGET.tar.gz"

    # A private or missing repository answers 404 to anonymous requests.
    check_repo() {
        fetch "https://api.github.com/repos/$REPO" >/dev/null 2>&1 ||
            die "cannot reach github.com/$REPO: the repository is private or does not exist"
    }

    # Explains a failed download: usually no (stable) release has been published yet.
    missing_release() {
        check_repo
        if [ -z "$VERSION" ] && [ "$PRE" = 0 ]; then
            NEWEST=$(fetch "https://api.github.com/repos/$REPO/releases?per_page=1" 2>/dev/null |
                sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' | head -n 1)
            if [ -n "$NEWEST" ]; then
                die "no stable release yet; the newest is pre-release $NEWEST. Install it with --pre"
            fi
        fi
        say "scat install: no release asset at $BASE/$ASSET" >&2
        say "build from source instead:" >&2
        say "  git clone https://github.com/$REPO && cd ${REPO#*/} && make install" >&2
        exit 1
    }

    if [ -n "$VERSION" ]; then
        BASE="https://github.com/$REPO/releases/download/$VERSION"
    elif [ "$PRE" = 1 ]; then
        check_repo
        TAG=$(fetch "https://api.github.com/repos/$REPO/releases?per_page=1" |
            sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' | head -n 1)
        [ -n "$TAG" ] || {
            say "scat install: $REPO has no releases yet; build from source:" >&2
            say "  git clone https://github.com/$REPO && cd ${REPO#*/} && make install" >&2
            exit 1
        }
        BASE="https://github.com/$REPO/releases/download/$TAG"
    else
        BASE="https://github.com/$REPO/releases/latest/download"
    fi

    TMP=$(mktemp -d)
    trap cleanup EXIT
    trap 'exit 1' INT TERM HUP

    say "downloading $ASSET"
    download "$BASE/$ASSET" "$TMP/$ASSET" 2>/dev/null || missing_release
    download "$BASE/$ASSET.sha256" "$TMP/$ASSET.sha256" || die "download failed: $BASE/$ASSET.sha256"

    EXPECTED=$(cut -d' ' -f1 "$TMP/$ASSET.sha256")
    ACTUAL=$(sha256 "$TMP/$ASSET")
    [ "$EXPECTED" = "$ACTUAL" ] || die "checksum mismatch: expected $EXPECTED, got $ACTUAL"

    tar -xzf "$TMP/$ASSET" -C "$TMP" scat
    if [ ! -f "$TMP/scat" ] || [ -L "$TMP/scat" ]; then
        die "the archive has no regular scat binary"
    fi
    mkdir -p "$INSTALL_DIR"
    # Stage under a fresh, unpredictable name next to the target (mktemp creates it
    # exclusively, so no existing file or symlink is followed), then rename, so a
    # running scat is replaced atomically.
    STAGED=$(mktemp "$INSTALL_DIR/.scat.XXXXXX") || die "cannot write to $INSTALL_DIR"
    cp "$TMP/scat" "$STAGED"
    chmod 755 "$STAGED"
    # Check that the new binary runs before it replaces the old one.
    NEW_VERSION=$("$STAGED" --version) || die "the downloaded binary does not run; kept the existing install"
    mv -f "$STAGED" "$INSTALL_DIR/scat"
    STAGED=""

    say "installed $NEW_VERSION to $INSTALL_DIR/scat"

    case ":$PATH:" in
        *":$INSTALL_DIR:"*) ;;
        *)
            say ""
            say "$INSTALL_DIR is not on your PATH. Add it:"
            say "  fish:      fish_add_path $INSTALL_DIR"
            say "  bash/zsh:  echo 'export PATH=\"$INSTALL_DIR:\$PATH\"' >> ~/.profile"
            ;;
    esac
    say "update later with: scat --update"
}

main "$@"
