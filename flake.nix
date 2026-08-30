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
          nativeBuildInputs = [ pkgs.pkg-config ];
          buildInputs = [ pkgs.libpulseaudio ];
          meta = {
            mainProgram = "sharkvis";
            description = "Terminal audio spectrum analyzer";
            homepage = "https://github.com/Matko802/sharkvis";
            license = pkgs.lib.licenses.mit;
            platforms = pkgs.lib.platforms.linux;
          };
        };

      overlay = final: _prev: {
        sharkvis = sharkvis { pkgs = final; };
      };
    in
    {
      packages = forAllSystems (pkgs: {
        default = sharkvis { inherit pkgs; };
        sharkvis = sharkvis { inherit pkgs; };
      });

      overlays.default = overlay;

      devShells = forAllSystems (pkgs:
        pkgs.mkShell {
          buildInputs = [
            pkgs.cargo
            pkgs.rustc
            pkgs.pkg-config
            pkgs.libpulseaudio
          ];
        });
    };
}