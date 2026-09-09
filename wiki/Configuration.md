# Configuration

Config is looked up in `$SHARKVIS_CONFIG`, then
`~/.config/sharkvis/config`, then `./config`. Settings changed in the
panel save automatically when you close it.

## Example

```ini
[general]
bars = 0
bar_width = 2
bar_spacing = 1
framerate = 60
sensitivity = 100
autosens = 1
lower_cutoff_freq = 50
higher_cutoff_freq = 8000

[smoothing]
noise_reduction = 0.20

[input]
method = pulse
source = auto
sample_rate = 48000
channels = 2

[color]
color_mode = 24bit
gradient_low = ffffff
gradient_high = ffffff

[visualizer]
mode = text
text = SHARKVIS
text_source = lyrics
text_align = center
text_size = 1
text_style = big ahh
provider = auto
offset_ms = 0

[lyrics]
folder = ~/Music

[mpris]
players = firefox,spotify
```

## General

| Key | Notes |
|-----|-------|
| `bars` | `0` means auto from width |
| `framerate` | Frames per second |
| `sensitivity` / `autosens` | Manual level, or let it ride the volume itself |
| `lower_cutoff_freq` / `higher_cutoff_freq` | Frequency window in Hz |
| `noise_reduction` | Smoothing amount |
| `source` | PulseAudio source, `auto` follows the default monitor |
| `channels` | `1` mono, `2` stereo (left/right react separately) |

## Color

| Key | Notes |
|-----|-------|
| `color_mode` | `"24bit"` or `"256"` |
| `gradient_low` / `gradient_high` | Hex colors, low end to high end |

## Visualizer

`mode` is one of `bars`, `wave`, `oscilloscope`, `text`. Switch it
live in the panel with `g`.

`text` renders the current lyric line (or the static `text`) in big
block letters. Each letter is lit by its own frequency bin exactly
like the bars: bin value drives letter brightness the way it drives
bar height. Each line runs its own gradient from `gradient_low`
(left) to `gradient_high` (right).

| Key | Notes |
|-----|-------|
| `text` | Static word when `text_source` isn't `lyrics` |
| `text_source` | `"lyrics"` follows the song, anything else shows `text` |
| `text_align` | `"left"` or `"center"` |
| `text_size` | `1`–`5` scales the letters, `0` means auto fit. An explicit size keeps its scale and crops to a window that follows the current line when taller than the screen |
| `text_style` | `"big ahh"` block letters (default), `"normal"` plain small terminal text with previous/next lines forced grey |

Anything outside the built-in Latin set renders through embedded GNU
Unifont bitmaps (CJK, Hangul, kana, Cyrillic, Greek, …). Accented
Latin folds to its base letter (`é` → `E`).

While lyrics load, text mode shows an 8-box cycling ring. Songs with
no lyrics anywhere show a dimmed `No Lyrics` line instead.

## Lyrics

Local `.lrc` files under `folder` win first (fuzzy matched on artist
plus title). Then the provider chain: [lrclib.net](https://lrclib.net)
(exact, then duration-scored search),
[Musixmatch](https://www.musixmatch.com) (anonymous token, no account
needed — true word-level Richsync timing when available), then
YouTube auto-captions via `yt-dlp` (from the player URL, else a
duration-guarded search).

Results cache per track in `~/.cache/sharkvis/lyrics/`. Tracks with
no lyrics anywhere are remembered for 7 days (no spinner, no
refetch); `r` forces a fresh check.

| Key | Notes |
|-----|-------|
| `folder` | Where your local `.lrc` files live |
| `provider` | `"auto"` scores every source per track and picks the best (default). A strong lrclib exact hit returns immediately. Set one explicitly to pin first-hit-wins order |
| `offset_ms` | Nudge sync, `-10000`–`10000` |
| `players` | Whitelist MPRIS players, first playing match wins (`playerctld` preferred). Empty means any |

Text-mode keys (lyrics showing): `s` manual search
(`Artist - Title`), `l` cycle media player, `r` force lyric reload,
`c` left/center align, `a` follow on/off, `p` switch provider
(auto/lrclib/musixmatch), `+`/`-` nudge sync ±500ms, `0` reset sync.

## Live state file

While running, sharkvis publishes color, levels and gradients ~20x
per second to `$XDG_RUNTIME_DIR/sharkvis/state` (fallback
`/tmp/sharkvis-$UID.state`) so tools like
[jefetch](https://github.com/Matko802/jefetch) follow instantly:

```text
color=#ff8800 energy=0.42 beat=1.00 color_low=#ffff00 color_high=#ff0000 bass=0.60 left=0.40 right=0.45
```

Files older than ~1s are stale. Set `SHARKVIS_NO_STATE=1` to
disable. Note: the monitor sees audio only — no song titles or
metadata.

## CLI Overrides

| Flag | What it does |
|------|--------------|
| `-p <path>` | Use this config file |
| `-h` | Print help |
| `-v` | Print version |
