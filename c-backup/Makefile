CC ?= cc
VERSION ?= 0.1.0
CFLAGS ?= -O2
CFLAGS += -std=c11 -Wall -Wextra -pedantic
CPPFLAGS += -DVERSION=\"$(VERSION)\"
PKG_CONFIG ?= pkg-config
CPPFLAGS += -D_POSIX_C_SOURCE=200809L
CPPFLAGS += $(shell $(PKG_CONFIG) --cflags libpulse-simple)
LDLIBS += $(shell $(PKG_CONFIG) --libs libpulse-simple) -lm -pthread
PREFIX ?= /usr/local

SRC := $(wildcard src/*.c)
OBJ := $(SRC:.c=.o)
BIN := sharkvis

all: $(BIN)

$(BIN): $(OBJ)
	$(CC) $(CFLAGS) $(LDFLAGS) -o $@ $(OBJ) $(LDLIBS)

src/%.o: src/%.c
	$(CC) $(CPPFLAGS) $(CFLAGS) -c -o $@ $<

compiledb:
	printf '[\n' > compile_commands.json
	CCWRAP_FRAG=$(abspath compile_commands.json) $(MAKE) clean all CC=$(CURDIR)/scripts/ccwrap
	sed -i '$$s/,$$//' compile_commands.json
	printf ']\n' >> compile_commands.json

install: $(BIN)
	install -Dm755 $(BIN) $(DESTDIR)$(PREFIX)/bin/$(BIN)

clean:
	rm -f $(BIN) $(OBJ)

# Install the build dependencies for the detected distro.
deps:
	@if command -v apt-get >/dev/null 2>&1; then \
		sudo apt-get install -y build-essential pkg-config libpulse-dev; \
	elif command -v pacman >/dev/null 2>&1; then \
		sudo pacman -S --needed base-devel libpulse; \
	elif command -v dnf >/dev/null 2>&1; then \
		sudo dnf install -y gcc make pkgconf-pkg-config pulseaudio-libs-devel; \
	elif command -v zypper >/dev/null 2>&1; then \
		sudo zypper install -y gcc make pkg-config libpulse-devel; \
	elif command -v xbps-install >/dev/null 2>&1; then \
		sudo xbps-install -S base-devel pkg-config pulseaudio-devel; \
	elif command -v apk >/dev/null 2>&1; then \
		sudo apk add build-base pkgconfig libpulse-dev; \
	elif command -v emerge >/dev/null 2>&1; then \
		sudo emerge --ask sys-devel/gcc sys-devel/make sys-devel/pkgconf media-libs/libpulse; \
	else \
		echo "Unsupported package manager. Install a C11 compiler, make, pkg-config"; \
		echo "and the libpulse-simple development headers for your distro."; \
	fi

.PHONY: all install clean deps
