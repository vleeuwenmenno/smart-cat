<img src="assets/icon.svg" width="64" height="64" alt="">

# scat

A smarter `cat`. In a terminal, `scat` renders what it prints: syntax-highlighted code, markdown, images, PDFs, audio and video info, and directory listings. When the output goes to a pipe or a file, `scat` behaves exactly like `cat`.

## Install

```sh
curl -fsSL https://raw.githubusercontent.com/vleeuwenmenno/smart-cat/main/install.sh | sh
```

This installs a static Linux binary (x86_64 or aarch64) to `~/.local/bin`. From a clone of this repository, `make install` does the same from source. To install pre-releases, add `-s -- --pre` after `sh`. To update:

```sh
scat --update
```

## Use

```sh
scat main.rs                # highlighted, with line numbers
scat song.flac              # audio tags and waveform; --play plays it with mpv
scat -p main.rs             # plain: no line numbers
scat README.md              # rendered markdown, wrapped to the terminal
scat                        # alone in a terminal: list the directory
scat data.csv               # a table fitted to the terminal; minified JSON is pretty-printed
scat clip.mp4               # video info (needs ffprobe)
scat photo.png              # the image itself (kitty, sixel or Unicode blocks)
scat doc.pdf --pages 2-4    # PDF info and pages; --text for the text
scat fullchain.pem          # certificate summaries and expiry; private keys stay hidden
scat app.log.gz             # decompressed, log levels coloured (diffs too: scat x.patch)
scat file.txt | grep foo    # piped: identical to cat
scat --diagnose             # what scat detected about your terminal
scat --init-config          # write ~/.config/scat/config.toml
```

All of `cat`'s flags work (`-n -b -s -A -v -E -T -e -t`). Shell completions: `scat --completions fish > ~/.config/fish/completions/scat.fish` (also bash, zsh).

## Docs

See [docs/](docs/README.md).

## License

[MIT](LICENSE)
