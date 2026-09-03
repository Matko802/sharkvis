VERSION ?= 0.1.0
PREFIX ?= /usr/local

# Build through cargo. On NixOS, run inside `nix develop` (which provides
# pkg-config and libpulseaudio) or build the flake (`nix build .#default`).
all: target/release/sharkvis

target/release/sharkvis: Cargo.toml Cargo.lock $(wildcard src/*.rs)
	cargo build --release

install: target/release/sharkvis
	install -Dm755 target/release/sharkvis $(DESTDIR)$(PREFIX)/bin/sharkvis

clean:
	cargo clean

# Install the build dependencies for the detected distro.
deps:
	@if command -v apt-get >/dev/null 2>&1; then \
		sudo apt-get install -y cargo rustc; \
	elif command -v pacman >/dev/null 2>&1; then \
		sudo pacman -S --needed rust cargo; \
	elif command -v dnf >/dev/null 2>&1; then \
		sudo dnf install -y cargo rust; \
	elif command -v zypper >/dev/null 2>&1; then \
		sudo zypper install -y cargo rust; \
	elif command -v xbps-install >/dev/null 2>&1; then \
		sudo xbps-install -S cargo rust; \
	elif command -v apk >/dev/null 2>&1; then \
		sudo apk add cargo rust; \
	elif command -v emerge >/dev/null 2>&1; then \
		sudo emerge --ask dev-lang/rust dev-lang/rust-bin; \
	else \
		echo "Unsupported package manager. Install cargo and rustc."; \
	fi

.PHONY: all install clean deps