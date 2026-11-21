# Installs the arena CLI from an unpacked release archive.
#
#   sudo make install
#   make install PREFIX=~/.local
#   sudo make uninstall      (use the same PREFIX)

PREFIX ?= /usr/local
DESTDIR ?=
BINDIR ?= $(PREFIX)/bin
DATADIR ?= $(PREFIX)/share
MANDIR ?= $(DATADIR)/man
BASHDIR ?= $(DATADIR)/bash-completion/completions
ZSHDIR ?= $(DATADIR)/zsh/site-functions
FISHDIR ?= $(DATADIR)/fish/vendor_completions.d

.PHONY: install uninstall

# `install -D` is avoided: BSD install on macOS doesn't have it.
install:
	install -d "$(DESTDIR)$(BINDIR)" "$(DESTDIR)$(MANDIR)/man1" \
		"$(DESTDIR)$(BASHDIR)" "$(DESTDIR)$(ZSHDIR)" "$(DESTDIR)$(FISHDIR)"
	install -m 755 arena "$(DESTDIR)$(BINDIR)/arena"
	install -m 644 man/*.1 "$(DESTDIR)$(MANDIR)/man1/"
	install -m 644 completions/arena.bash "$(DESTDIR)$(BASHDIR)/arena"
	install -m 644 completions/_arena "$(DESTDIR)$(ZSHDIR)/_arena"
	install -m 644 completions/arena.fish "$(DESTDIR)$(FISHDIR)/arena.fish"

uninstall:
	rm -f "$(DESTDIR)$(BINDIR)/arena"
	for page in man/*.1; do rm -f "$(DESTDIR)$(MANDIR)/man1/$${page##*/}"; done
	rm -f "$(DESTDIR)$(BASHDIR)/arena"
	rm -f "$(DESTDIR)$(ZSHDIR)/_arena"
	rm -f "$(DESTDIR)$(FISHDIR)/arena.fish"
