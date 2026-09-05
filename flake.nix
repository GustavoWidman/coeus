{
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
    fenix = {
      url = "github:nix-community/fenix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    crane.url = "github:ipetkov/crane";
    flake-parts.url = "github:hercules-ci/flake-parts";
    systems.url = "github:nix-systems/default";
    devshell.url = "github:numtide/devshell";
  };

  outputs =
    inputs@{
      flake-parts,
      crane,
      fenix,
      systems,
      devshell,
      ...
    }:
    flake-parts.lib.mkFlake { inherit inputs; } {
      systems = import systems;
      imports = [
        devshell.flakeModule
      ];
      perSystem =
        {
          pkgs,
          system,
          lib,
          ...
        }:
        let
          toolchain = fenix.packages.${system}.fromToolchainFile {
            file = ./rust-toolchain.toml;
            sha256 = "sha256-3MyLNjhfHtXhOj0hzUe3wrwfX6h7B3JURIfNHuSu19w=";
          };

          craneLib = (crane.mkLib pkgs).overrideToolchain toolchain;
          root = ./.;

          args = {
            src = lib.fileset.toSource {
              inherit root;
              fileset = lib.fileset.unions [
                (craneLib.fileset.commonCargoSources root)
                # (lib.fileset.fileFilter (file: file.hasExt "md") root)
              ];
            };
            strictDeps = true;

            nativeBuildInputs = [ ];
            buildInputs = [ ];
          };

          bin = craneLib.buildPackage (
            args
            // {
              cargoArtifacts = craneLib.buildDepsOnly args;
            }
          );
        in
        {
          # checks.<package> = bin;

          packages.default = bin;

          devshells.default = {
            packages = [
              toolchain
            ];

            motd = "";
          };
        };
    };
}
