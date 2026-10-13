{
  description = "A terminal UI that keeps AI-readable documentation of a codebase current";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs =
    { self, nixpkgs }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "aarch64-darwin"
      ];
      forAllSystems = f: nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});
      workspace = (builtins.fromTOML (builtins.readFile ./Cargo.toml)).workspace.package;
    in
    {
      packages = forAllSystems (pkgs: {
        default = pkgs.rustPlatform.buildRustPackage {
          pname = "warlock";
          inherit (workspace) version;

          src = pkgs.lib.fileset.toSource {
            root = ./.;
            fileset = pkgs.lib.fileset.unions [
              ./Cargo.toml
              ./Cargo.lock
              ./crates
            ];
          };
          cargoLock.lockFile = ./Cargo.lock;

          cargoBuildFlags = [ "--package" "warlock-tui" ];
          cargoTestFlags = [ "--workspace" ];
          nativeCheckInputs = [ pkgs.git ];

          meta = {
            description = "A terminal UI that keeps AI-readable documentation of a codebase current";
            homepage = "https://github.com/Genetic-Pottery/warlock";
            license = pkgs.lib.licenses.asl20;
            mainProgram = "warlock";
            platforms = systems;
          };
        };
      });
    };
}
