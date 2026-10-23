.DEFAULT_GOAL := help

CARGO ?= cargo
PREFIX ?= $(HOME)/.local
BINDIR ?= $(PREFIX)/bin
TARGET ?= $(shell uname -m)-unknown-linux-musl
VERSION ?=

# C compiler for the static musl build (the Oniguruma regex library is C).
# musl-gcc is right; plain gcc also works when building for the host's own CPU.
TARGET_ARCH := $(firstword $(subst -, ,$(TARGET)))
MUSL_GCC := $(shell command -v musl-gcc 2>/dev/null)
ifneq ($(MUSL_GCC),)
DIST_CC ?= musl-gcc
else ifeq ($(TARGET_ARCH),$(shell uname -m))
DIST_CC ?= gcc
endif
DIST_CC_VAR := CC_$(subst -,_,$(TARGET))

.PHONY: help install uninstall run build release test lint fmt fmt-check shellcheck check dist bump

help:
	@if [ -t 1 ] && [ -z "$${NO_COLOR:-}" ]; then \
		H=$$(printf '\033[1;33m'); T=$$(printf '\033[1;36m'); D=$$(printf '\033[2m'); R=$$(printf '\033[0m'); \
	else H=; T=; D=; R=; fi; \
	row() { printf "  $${T}%-26s$${R} %s" "$$1" "$$2"; [ -z "$${3:-}" ] || printf " $${D}%s$${R}" "$$3"; printf "\n"; }; \
	printf "\n$${H}scat$${R}: a smarter cat\n"; \
	printf "\n$${H}Start here$${R}\n"; \
	row "make install" "Build, install to $(BINDIR), add completions" "(~30 s)"; \
	row "make run ARGS=\"-n FILE\"" "Run from source"; \
	row "make uninstall" "Remove scat, its completions and man page"; \
	printf "\n$${H}Develop$${R}\n"; \
	row "make test" "Run all tests" "(~10 s)"; \
	row "make check" "Everything CI runs: format, lint, tests"; \
	row "make fmt" "Format the code"; \
	row "make build" "Debug build"; \
	printf "\n$${H}Release$${R}\n"; \
	row "make dist" "Static archive in dist/ (binary, completions, man page)" "(TARGET=$(TARGET))"; \
	row "make bump VERSION=x.y.z" "Commit and tag a release" "(x.y.z-rc.1 = pre-release)"; \
	printf "\n$${D}Docs: docs/README.md$${R}\n\n"

install: release
	install -Dm755 target/release/scat "$(BINDIR)/scat"
	@echo "installed $$("$(BINDIR)/scat" --version) to $(BINDIR)/scat"
	@"$(BINDIR)/scat" --install-completions
	@case ":$$PATH:" in *":$(BINDIR):"*) ;; *) echo "$(BINDIR) is not on PATH (fish: fish_add_path $(BINDIR))";; esac

uninstall:
	@if [ -x "$(BINDIR)/scat" ]; then "$(BINDIR)/scat" --uninstall-completions; fi
	rm -f "$(BINDIR)/scat"

run:
	$(CARGO) run --locked -- $(ARGS)

build:
	$(CARGO) build --locked

release:
	$(CARGO) build --release --locked

test:
	$(CARGO) test --locked

lint:
	$(CARGO) clippy --locked --all-targets -- -D warnings

fmt:
	$(CARGO) fmt

fmt-check:
	$(CARGO) fmt --check

shellcheck:
	shellcheck install.sh

check: fmt-check lint test

# Static release archive, as published on GitHub: dist/scat-$(TARGET).tar.gz + .sha256
dist:
	@test -n "$(DIST_CC)" || { echo "no C compiler for $(TARGET): install musl (musl-gcc) or pass DIST_CC=..."; exit 1; }
	@if [ "$(DIST_CC)" = gcc ]; then echo "note: musl-gcc not found, building with gcc"; fi
	@if command -v rustup >/dev/null 2>&1; then rustup target add $(TARGET) >/dev/null; fi
	@# SQLITE_DISABLE_LFS: plain open/stat names; the host-gcc fallback has no musl open64 and friends.
	env $(DIST_CC_VAR)=$(DIST_CC) LIBSQLITE3_FLAGS=-DSQLITE_DISABLE_LFS $(CARGO) build --release --locked --target $(TARGET)
	mkdir -p dist/stage
	cp target/$(TARGET)/release/scat README.md LICENSE dist/stage/
	@# Completions and the man page come from the binary itself, so only a native build can make them.
	@if [ "$(TARGET_ARCH)" = "$$(uname -m)" ]; then \
		mkdir -p dist/stage/completions; \
		for shell in bash fish zsh; do target/$(TARGET)/release/scat --completions $$shell > dist/stage/completions/scat.$$shell || exit 1; done; \
		target/$(TARGET)/release/scat --man > dist/stage/scat.1 || exit 1; \
	else echo "note: cross build, no completions or man page in the archive"; fi
	tar -czf dist/scat-$(TARGET).tar.gz -C dist/stage $$(cd dist/stage && ls)
	cd dist && sha256sum scat-$(TARGET).tar.gz > scat-$(TARGET).tar.gz.sha256
	rm -rf dist/stage

# Set the version, commit and tag. Push with: git push --follow-tags
bump:
	@test -n "$(VERSION)" || { echo "usage: make bump VERSION=x.y.z[-rc.N]"; exit 1; }
	@git diff --quiet && git diff --cached --quiet || { echo "working tree not clean"; exit 1; }
	sed -i 's/^version = ".*"/version = "$(VERSION)"/' Cargo.toml
	$(CARGO) update --workspace --offline
	git commit -am "chore: release v$(VERSION)"
	git tag -a "v$(VERSION)" -m "v$(VERSION)"
