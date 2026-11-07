PREFIX ?= $(HOME)/.local
BINDIR ?= $(PREFIX)/bin
DATADIR ?= $(if $(XDG_DATA_HOME),$(XDG_DATA_HOME),$(PREFIX)/share)
CARGO ?= cargo

.PHONY: build install

build:
	$(CARGO) build --release --locked

install: build
	desktop-file-validate assets/reel.desktop
	install -Dm755 target/release/reel "$(BINDIR)/reel"
	install -Dm644 assets/reel.svg "$(DATADIR)/icons/hicolor/scalable/apps/reel.svg"
	desktop-file-install --dir="$(DATADIR)/applications" \
		--set-key=Exec --set-value='"$(abspath $(BINDIR))/reel" %u' \
		--set-key=TryExec --set-value="$(abspath $(BINDIR))/reel" \
		assets/reel.desktop
	update-desktop-database "$(DATADIR)/applications"
	gtk-update-icon-cache --force --ignore-theme-index "$(DATADIR)/icons/hicolor"
