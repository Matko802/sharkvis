VERSION ?= 0.2.9
PREFIX ?= /usr/local
BUILD ?= build
CC ?= gcc
CFLAGS ?= -O2 -std=c17 -D_POSIX_C_SOURCE=200809L -D_DEFAULT_SOURCE -Wall -Wextra

SRCS = src/main.c src/common.c src/term.c src/config.c src/fft.c \
	src/dsp.c src/pulse.c src/audio.c src/raw.c src/state.c \
	src/render.c src/settings.c src/lyrics.c src/mpris.c \
	src/musixmatch.c src/unifont.c $(BUILD)/unifont_data.c

all: $(BUILD)/sharkvis

$(BUILD)/unifont_data.c $(BUILD)/unifont_data.h: scripts/unifont_hex_to_c.py third_party/unifont.hex
	mkdir -p $(BUILD)
	python3 scripts/unifont_hex_to_c.py third_party/unifont.hex $(BUILD)/unifont_data.c $(BUILD)/unifont_data.h

$(BUILD)/sharkvis: $(SRCS) src/*.h
	$(CC) $(CFLAGS) -DSHARKVIS_VERSION='"$(VERSION)"' -Isrc -I$(BUILD) $(SRCS) -o $@ -lm -pthread

cmake-build:
	mkdir -p $(BUILD)/cmake && cd $(BUILD)/cmake && cmake ../.. -DCMAKE_BUILD_TYPE=Release && cmake --build .

install: all
	install -Dm755 $(BUILD)/sharkvis $(DESTDIR)$(PREFIX)/bin/sharkvis

clean:
	rm -rf $(BUILD) $(BUILD)/cmake

uninstall:
	rm -f $(DESTDIR)$(PREFIX)/bin/sharkvis

deps:
	@if command -v apt-get >/dev/null 2>&1; then \
		sudo apt-get install -y gcc cmake make python3; \
	elif command -v pacman >/dev/null 2>&1; then \
		sudo pacman -S --needed gcc cmake make python; \
	elif command -v dnf >/dev/null 2>&1; then \
		sudo dnf install -y gcc cmake make python3; \
	elif command -v zypper >/dev/null 2>&1; then \
		sudo zypper install -y gcc cmake make python3; \
	elif command -v xbps-install >/dev/null 2>&1; then \
		sudo xbps-install -S gcc cmake gmake python3; \
	elif command -v apk >/dev/null 2>&1; then \
		sudo apk add gcc cmake make python3; \
	elif command -v emerge >/dev/null 2>&1; then \
		sudo emerge --ask sys-devel/gcc dev-build/cmake dev-build/make dev-lang/python; \
	else \
		echo "Unsupported package manager. Install gcc, cmake, make and python3."; \
	fi

.PHONY: all cmake-build install uninstall clean deps
