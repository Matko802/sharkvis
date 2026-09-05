<div align="center">

<img src="Logo/sharkvis.png" width="120" alt="sharkvis logo :3" />

# sharkvis

Linux only audio visualizer, now in Rust

Inspired by <sub>[cava](https://github.com/karlstav/cava)</sub> and <sub>[cli-visualizer](https://github.com/PosixAlchemist/cli-visualizer)</sub>

</div>

## Features

- PulseAudio / PipeWire support
- Smoothness adjust, noise reduction
- Autosensitivity, manual sensitivity control, adjustable cutoff frequencies
- TUI settings
- Color customization
- Pure Rust
- Uses Musl

## Building

```sh
git clone https://github.com/Matko802/sharkvis.git
cd sharkvis
make deps
make
sudo make install
```
## Updating it

```sh
cd ../sharkvis && git pull && ./build.sh && sudo make install
```

## Usage

```sh
sharkvis
sharkvis -p config.conf
sharkvis -h
```

| Key | Action |
| --- | --- |
| `g` | open settings |
| `q` / `Ctrl-C` | quit |

The config file is looked up in `$SHARKVIS_CONFIG`, then
`~/.config/sharkvis/config`, then `./config`. Settings changed in the panel
are saved automatically when you close the panel or quit.

## Update

Nix flake:

```sh
cd ~/fish-flake
nix flake update sharkvis; nh os switch -H machine1
```

Standalone:

```sh
cd sharkvis && git pull; make && sudo make install
```

Restart running copies after updating (`q` to quit, relaunch).

## Live state file (jefetch integration)

While running, sharkvis publishes color, levels and gradients ~20x per
second to `$XDG_RUNTIME_DIR/sharkvis/state`
(fallback `/tmp/sharkvis-$UID.state`) so tools like
[jefetch](https://github.com/Matko802/jefetch) follow instantly, without
waiting for the settings panel to close:

```text
color=#ff8800 energy=0.42 beat=1.00 color_low=#ffff00 color_high=#ff0000 bass=0.60 left=0.40 right=0.45
```

- `color` — gradient color at the current volume (`#rrggbb`)
- `energy` — overall volume, mean bar height `0..1`
- `beat` — beat envelope `0..1`: kicks, snares and other onsets lock
  a tempo grid that keeps pulsing through soft hits, recalibrates on
  tempo changes, and drops after unsupported bars
- `bass`, `left`, `right` — bass, left and right channel means `0..1`

Files older than ~1s are stale. Set `SHARKVIS_NO_STATE=1` to disable.
Note: the monitor sees audio only — no song titles or metadata.

## Any distro with Nix:

```sh
nix develop   # drop into a shell with cargo
make          # build inside the dev shell
```
Or
```sh
nix run github:Matko802/sharkvis
```

## As a flake input

```nix
{
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
    sharkvis = {
      url = "github:Matko802/sharkvis";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { nixpkgs, sharkvis, ... }: {
    packages.x86_64-linux.default = sharkvis.packages.x86_64-linux.default;
  };
}
```

## As an overlay

```nix
{
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
    sharkvis = {
      url = "github:Matko802/sharkvis";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    { nixpkgs, sharkvis, ... }:
    let
      system = "x86_64-linux";
    in
    {
      nixosConfigurations.myhost = nixpkgs.lib.nixosSystem {
        inherit system;
        modules = [
          {
            nixpkgs.overlays = [ sharkvis.overlays.default ];
            environment.systemPackages = [ sharkvis.packages.${system}.default ];
          }
        ];
      };
    };
}
```

## Standalone build from source

```sh
nix build github:Matko802/sharkvis
nix run github:Matko802/sharkvis
```

## Develop

```sh
nix develop github:Matko802/sharkvis
```

## License

This project is released under the MIT License. See [LICENSE](https://github.com/Matko802/sharkvis/blob/main/LICENSE).
