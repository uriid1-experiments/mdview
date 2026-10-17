# mdview — Markdown viewer/editor on GitHub's rendering stack.
#
#   make              release build
#   make run FILE=x   run on a file
#   make check        unit tests + smoke test of the release binary
#   make install      install to ~/.local (PREFIX=/usr for system-wide, DESTDIR for packaging)
#   make help         list targets

CARGO   ?= cargo
PREFIX  ?= $(HOME)/.local
DESTDIR ?=
FILE    ?= samples/demo.md
KATEX   ?= 0.16
MERMAID ?= 11

BIN      = target/release/mdview
VERSION  = $(shell sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml)
DIST     = mdview-$(VERSION)-linux-$(shell uname -m)

.PHONY: all build release run test check lint fmt install uninstall default-app \
        vendor vendor-update syntaxes dump dist tag clean help

all: release

build:            ## Debug build
	$(CARGO) build

release:          ## Optimized build → target/release/mdview
	$(CARGO) build --release

run: release      ## Run on FILE (default: samples/demo.md)
	$(BIN) $(FILE)

test:             ## Unit tests (render pipeline)
	$(CARGO) test

check: test release ## Tests + smoke test of the release binary
	$(BIN) --dump samples/demo.md | grep -q 'class="pl-'
	$(BIN) --dump samples/demo.md | grep -q 'markdown-alert'
	$(BIN) --dump samples/demo.md | grep -q 'data-math-style'
	@echo "check: OK"

lint:             ## cargo clippy, warnings are errors
	$(CARGO) clippy --all-targets -- -D warnings

fmt:              ## cargo fmt
	$(CARGO) fmt

install: release  ## Install binary, .desktop entry and icon under PREFIX
	install -Dm755 $(BIN) $(DESTDIR)$(PREFIX)/bin/mdview
	install -Dm644 assets/mdview.svg $(DESTDIR)$(PREFIX)/share/icons/hicolor/scalable/apps/mdview.svg
	install -d $(DESTDIR)$(PREFIX)/share/applications
	sed 's|@BIN@|$(PREFIX)/bin/mdview|' mdview.desktop.in > $(DESTDIR)$(PREFIX)/share/applications/mdview.desktop
	-update-desktop-database $(DESTDIR)$(PREFIX)/share/applications 2>/dev/null
	@echo "installed: $(PREFIX)/bin/mdview"

uninstall:        ## Remove what install put under PREFIX
	rm -f $(DESTDIR)$(PREFIX)/bin/mdview \
	      $(DESTDIR)$(PREFIX)/share/applications/mdview.desktop \
	      $(DESTDIR)$(PREFIX)/share/icons/hicolor/scalable/apps/mdview.svg
	-update-desktop-database $(DESTDIR)$(PREFIX)/share/applications 2>/dev/null

default-app:      ## Make mdview the default application for Markdown files
	xdg-mime default mdview.desktop text/markdown text/x-markdown

vendor:           ## Regenerate src/vendor.rs from assets/vendor
	python3 scripts/vendor.py

vendor-update:    ## Download KaTeX/Mermaid (KATEX=0.16 MERMAID=11 version specs) and regenerate
	python3 scripts/vendor.py update $(KATEX) $(MERMAID)

syntaxes: release ## List highlighting languages
	$(BIN) --syntaxes

dump: release     ## Print rendered HTML of FILE
	$(BIN) --dump $(FILE)

dist: release     ## Tarball with binary, desktop entry, icon and README
	rm -rf dist/$(DIST) && mkdir -p dist/$(DIST)
	cp $(BIN) README.md README.ru.md mdview.desktop.in assets/mdview.svg dist/$(DIST)/
	cp Makefile dist/$(DIST)/
	tar -C dist -czf dist/$(DIST).tar.gz $(DIST)
	@echo "dist/$(DIST).tar.gz"

tag:              ## git tag v$(VERSION) (version from Cargo.toml)
	git tag -a v$(VERSION) -m "mdview $(VERSION)"
	@echo "tagged v$(VERSION); push with: git push --tags"

clean:            ## cargo clean + dist/
	$(CARGO) clean
	rm -rf dist

help:             ## This list
	@grep -E '^[a-zA-Z_-]+:.*## ' $(MAKEFILE_LIST) | awk 'BEGIN{FS=":.*## "}{printf "  %-16s %s\n", $$1, $$2}'
