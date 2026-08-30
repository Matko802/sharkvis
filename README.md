<div align="center">

<img src="Logo/sharkvis.png" width="120" alt="sharkvis logo :3" />

# sharkvis

Linux only audio visualizer, now in Rust

Inspired by <sub>[cava](https://github.com/karlstav/cava)</sub> and <sub>[cli-visualizer](https://github.com/PosixAlchemist/cli-visualizer)</sub>

</div>

## Features

- PulseAudio / PipeWire support
- smoothness adjust, noise reduction
- autosensitivity, manual sensitivity control, adjustable cutoff frequencies
- TUI settings
- color customization

## Building

Rust (cargo) and the libpulse-simple development headers are required.

```sh
git clone https://github.com/Matko802/sharkvis.git
cd sharkvis
make deps      
make           
sudo make install
```

On NixOS or any distro with Nix, prefer the flake (no system packages needed):

```sh
nix develop   # drop into a shell with cargo, pkg-config and libpulse
make          # build inside the dev shell
```

Prefer not to touch your system at all?

```sh
nix run github:Matko802/sharkvis
```

To install somewhere else instead of `/usr/local`:

```sh
make PREFIX=$HOME/.local install
```

### Run it

```sh
sharkvis
```

## Usage

```sh
sharkvis
sharkvis -p config.conf
sharkvis -h
```

Keys:

| Key               | Action                          |
| ----------------- | ------------------------------- |
| `g`               | open settings                   |
| `q` / `Ctrl-C`    | quit                            |

The config file is looked up in `$SHARKVIS_CONFIG`, then
`~/.config/sharkvis/config`, then `./config`. Settings changed in the panel
are saved automatically when you close the panel or quit.

## Nix flakes

sharkvis ships with its own flake, so you can pull it straight from GitHub.

### As a flake input

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

### As an overlay

The flake also exposes `overlays.default`, so you can enable it with
`nixpkgs.overlays = [ sharkvis.overlays.default ];` and get `pkgs.sharkvis`.

A full NixOS example that pulls the flake in as both an overlay and a package:

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

### Standalone build from source

```sh
nix build github:Matko802/sharkvis
nix run github:Matko802/sharkvis
```

### Development

```sh
nix develop github:Matko802/sharkvis  
```

## License

This project is released under the MIT License. See [LICENSE.txt](https://github.com/Matko802/sharkvis/blob/main/LICENSE).
