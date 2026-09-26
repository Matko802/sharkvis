# Configuration

Config is looked up in `$SHARKVIS_CONFIG`, then
`~/.config/sharkvis/config.jsonc`, then `~/.config/sharkvis/config.toml`,
then `./config.jsonc`, then `./config.toml`. JSONC (JSON with `//` and
`/* */` comments, same as jefetch) is preferred for new files; TOML still
loads. Settings changed in the panel save automatically when you close
them, keeping the file's format.

## Example (JSONC)

```jsonc
{
    // comments allowed
    "general": {
        "bars": 0,
        "bar_width": 2,
        "bar_spacing": 1,
        "framerate": 60,
        "sensitivity": 100,
        "autosens": true,
        "lower_cutoff_freq": 50,
        "higher_cutoff_freq": 8000
    },
    "smoothing": {
        "noise_reduction": 0.20
    },
    "input": {
        "method": "pulse", // or "pipewire", "auto"
        "source": "auto",
        "sample_rate": 48000,
        "channels": 2
    },
    "color": {
        "color_mode": "24bit", // or "256"
        "gradient_low": "ffffff",
        "gradient_high": "ffffff"
    },
    "visualizer": {
        "mode": "lyrics",
        "chars": "▁▂▃▄▅▆▇█",
        "text_align": "center",
        "text_size": 1,
        "text_style": "big ahh",
        "provider": "auto",
        "offset_ms": 0
    },
    "lyrics": {
        "folder": "~/Music"
    },
    "mpris": {
        "players": "firefox,spotify"
    }
}
```

## Example (TOML, legacy)

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
mode = lyrics
chars = ▁▂▃▄▅▆▇█
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
| `gradient_low` / `gradient_high` | Terminal color name (`red`, `blue`, `green`, `yellow`, `magenta`, `cyan`, `white`, `black`, `gray`, `orange`, `purple`, `lime`, `teal`, `pink`) or hex (`ff0000`). Names emit terminal colors so they follow the terminal theme, hex emits fixed RGB |

## Visualizer

`mode` is one of `bars`, `wave`, `oscilloscope`, `lyrics`. Switch it
live in the panel with `g`. (`text` still works in old configs as an
alias for `lyrics`.)

`lyrics` renders the current lyric line in big block letters at a
fixed brightness — it is not audio-visualized. Each line runs its own
gradient from `gradient_low` (left) to `gradient_high` (right).

| Key | Notes |
|-----|-------|
| `chars` | Bar symbols, low to high (tools like jefetch mimic these with `chars=sharkvis`) |
| `text_align` | `"left"` or `"center"` |
| `text_size` | `1`–`5` scales the letters, `0` means auto fit. An explicit size keeps its scale and crops to a window that follows the current line when taller than the screen |
| `text_style` | `"big ahh"` block letters (default), `"normal"` plain small terminal text with previous/next lines forced grey |

Anything outside the built-in Latin set renders through embedded GNU
Unifont bitmaps (CJK, Hangul, kana, Cyrillic, Greek, …). Accented
Latin folds to its base letter (`é` → `E`).

While lyrics load, lyrics mode shows an 8-box cycling ring. Songs with
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

Lyrics-mode keys: `s` manual search
(`Artist - Title`), `l` cycle media player, `r` force lyric reload,
`c` left/center align, `a` follow on/off, `p` switch provider
(auto/lrclib/musixmatch), `+`/`-` nudge sync ±500ms, `0` reset sync.

## Live state file

While running, sharkvis publishes color, levels and gradients ~20x
per second to `$XDG_RUNTIME_DIR/sharkvis/state` (fallback
`/tmp/sharkvis-$UID.state`) so tools like
[jefetch](https://github.com/Matko802/jefetch) follow instantly:

```text
color=#ff8800 energy=0.42 beat=1.00 color_low=#ffff00 color_high=#ff0000 bass=0.60 left=0.40 right=0.45 started=1757917315000 pid=1234
```

Each instance also writes its own `state-<pid>` file next to it, stamped
with `started=` (when it was opened) and `pid=`. With several sessions
running, consumers follow only the newest one and ignore older ones.

Files older than ~1s are stale. Session files are removed on exit (and
stale leftovers from crashed runs are dropped at startup), so consumers
never show frozen colors from a dead instance. Set `SHARKVIS_NO_STATE=1`
to disable. Note: the monitor sees audio only — no song titles or
metadata.

## CLI Overrides

| Flag | What it does |
|------|--------------|
| `-p <path>` | Use this config file |
| `-h` | Print help |
| `-v` | Print version |
