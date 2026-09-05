#!/usr/bin/env bash
# Install sharkvis from GitHub Releases (static binary, any Linux distro).
# Needs a running PulseAudio or PipeWire-pulse server for audio capture.
# Usage: curl -fsSL https://raw.githubusercontent.com/Matko802/sharkvis/main/install.sh | sh
set -euo pipefail

REPO="Matko802/sharkvis"

if [ "$(uname -s)" != "Linux" ]; then
    echo "error: Linux only" >&2
    exit 1
fi
case "$(uname -m)" in
    x86_64) ARCH="x86_64" ;;
    aarch64 | arm64) ARCH="aarch64" ;;
    *) echo "error: unsupported architecture $(uname -m)" >&2; exit 1 ;;
esac

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

echo "Downloading sharkvis (linux-$ARCH)..."
curl -fsSL "https://github.com/$REPO/releases/latest/download/sharkvis-linux-$ARCH.tar.gz" \
    | tar -xz -C "$TMP"

if [ -w /usr/local/bin ]; then
    install -Dm755 "$TMP/sharkvis" /usr/local/bin/sharkvis
else
    sudo install -Dm755 "$TMP/sharkvis" /usr/local/bin/sharkvis
fi

echo "Installed: $(command -v sharkvis)"
