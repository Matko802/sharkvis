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
        pkgs.stdenv.mkDerivation {
          pname = "sharkvis";
          version = "0.1.0";
          src = pkgs.lib.cleanSource ./.;
          nativeBuildInputs = [ pkgs.cmake pkgs.python3 ];
          cmakeFlags = [ "-DCMAKE_BUILD_TYPE=Release" ];
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
      packages = forAllSystems (pkgs:
        let
          build = sharkvis { pkgs = pkgs.pkgsStatic; };
        in
        {
          default = pkgs.runCommand "sharkvis" { } ''
            mkdir -p $out/bin
            install -Dm755 ${build}/bin/sharkvis $out/bin/sharkvis
          '';
          sharkvis = pkgs.runCommand "sharkvis" { } ''
            mkdir -p $out/bin
            install -Dm755 ${build}/bin/sharkvis $out/bin/sharkvis
          '';
        });

      overlays.default = final: _prev: {
        sharkvis = sharkvis { pkgs = final.pkgsStatic; };
      };

      devShells = forAllSystems (pkgs:
        pkgs.mkShell {
          buildInputs = [ pkgs.gcc pkgs.cmake pkgs.gnumake pkgs.python3 ];
        });
    };
}
