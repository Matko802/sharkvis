<div align="center">

<img src="Logo/sharkvis.png" width="120" alt="sharkvis logo :3" />

# sharkvis

Linux only audio visualizer, now in Rust

Inspired by [cava](https://github.com/karlstav/cava) and [cli-visualizer](https://github.com/PosixAlchemist/cli-visualizer) also [LyricsMPRIS-Rust](https://github.com/BEST8OY/LyricsMPRIS-Rust)

</div>

## Features

- PulseAudio / PipeWire support
- Autosensitivity, smoothing, adjustable cutoffs
- Synced lyrics with text mode
- integrated with [jefetch](https://github.com/Matko802/jefetch)

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
cd sharkvis && git pull && sudo make install
```
## Usage

<div align="center">
  <a href="./wiki/Configuration.md"><b>📖 Configuration wiki</b></a>
</div>

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
`~/.config/sharkvis/config.toml`, then `./config.toml`

## Updating

```sh
cd sharkvis && git pull && sudo make install
```

## Any distro with Nix:

```sh
nix run github:Matko802/sharkvis
```

### As flake input

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

### As overlay

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

## License

This project is released under the MIT License. See [LICENSE](https://github.com/Matko802/sharkvis/blob/main/LICENSE).
