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
    flake-parts.lib.mkFlake { inherit inputs; } (
      { self, flake-parts-lib, ... }:
      let
        common = flake-parts-lib.importApply ./nix/modules/common.nix { inherit self; };
      in
      {
        systems = import systems;
        imports = [
          devshell.flakeModule
        ];

        flake = {
          nixosModules.default.imports = [
            common
            ./nix/modules/nixos.nix
          ];
          darwinModules.default.imports = [
            common
            ./nix/modules/darwin.nix
          ];
        };

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
            coeus = pkgs.callPackage ./nix/package.nix { inherit craneLib; };
          in
          {
            packages.default = coeus;

            checks = {
              inherit coeus;
            }
            // coeus.passthru.checks
            // lib.optionalAttrs pkgs.stdenv.hostPlatform.isLinux {
              nixos = pkgs.testers.runNixOSTest (import ./nix/test.nix { inherit self; });
            };

            devshells.default = {
              packages = [
                toolchain
              ];

              motd = "";
            };
          };
      }
    );
}
