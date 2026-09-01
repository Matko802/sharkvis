{
  description = "sharkvis - terminal audio spectrum analyzer";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
  };

  outputs =
    { self, nixpkgs }:
    let
      systems = [ "x86_64-linux" "aarch64-linux" ];
      forAllSystems = f: nixpkgs.lib.genAttrs systems (system: f (nixpkgs.legacyPackages.${system}));

      sharkvis =
        { pkgs }:
        pkgs.rustPlatform.buildRustPackage {
          pname = "sharkvis";
          version = "0.1.0";
          src = pkgs.lib.cleanSource ./.;
          cargoLock.lockFile = ./Cargo.lock;
          meta = {
            mainProgram = "sharkvis";
            description = "Terminal audio spectrum analyzer";
            homepage = "https://github.com/Matko802/sharkvis";
            license = pkgs.lib.licenses.mit;
            platforms = pkgs.lib.platforms.linux;
          };
        };
    in
    {
      # Fully static musl build: no glibc, no libpulse linked in. PulseAudio
      # client libs are loaded at runtime with dlopen on Windows-of-things that
      # provide them (pulseaudio, pipewire-pulse).
      packages = forAllSystems (pkgs: {
        default = sharkvis { pkgs = pkgs.pkgsStatic; };
        sharkvis = sharkvis { pkgs = pkgs.pkgsStatic; };
      });

      overlays.default = final: _prev: {
        sharkvis = sharkvis { pkgs = final.pkgsStatic; };
      };

      devShells = forAllSystems (pkgs:
        pkgs.mkShell {
          buildInputs = [ pkgs.pkgsMusl.cargo pkgs.pkgsMusl.rustc ];
        });
    };
}