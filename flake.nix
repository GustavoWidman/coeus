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
    # Used for genuine Darwin module evaluation, not a runtime dependency.
    nix-darwin = {
      url = "github:nix-darwin/nix-darwin/4cff07de74b50e64bdd68cd4e722ab5b6b35ee48";
      inputs.nixpkgs.follows = "nixpkgs";
    };
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
      # This pinned Nixpkgs 26.11 no longer supports Intel Darwin.
      systems = builtins.filter (system: system != "x86_64-darwin") (import systems);
      imports = [
        devshell.flakeModule
      ];
      flake = {
        nixosModules.coeusd = import ./nix/modules/nixos.nix inputs.self;
        nixosModules.default = inputs.self.nixosModules.coeusd;
        darwinModules.coeusd = import ./nix/modules/darwin.nix inputs.self;
        darwinModules.default = inputs.self.darwinModules.coeusd;
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
          root = ./.;

          args = {
            src = lib.fileset.toSource {
              inherit root;
              fileset = lib.fileset.unions [
                (craneLib.fileset.commonCargoSources root)
                ./config.client.default.toml
                ./config.server.default.toml
                ./nix/elevation.py
              ];
            };
            strictDeps = true;

            nativeBuildInputs = [
              pkgs.perl
              pkgs.python3
            ];
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
          packages.default = bin;
          packages.coeus = bin;
          checks = {
            package = bin;
            modules = import ./nix/tests/modules.nix { inherit inputs pkgs system; };
            client-config = import ./nix/tests/home-manager.nix { inherit inputs pkgs system; };
            elevation = pkgs.runCommand "coeus-elevation-tests" { nativeBuildInputs = [ pkgs.python3 ]; } ''
              python3 ${./nix/tests/elevation.py} ${./nix/elevation.py}
              python3 ${./nix/elevation_supervision_tests.py} ${./nix/elevation.py}
              touch $out
            '';
          };

          devshells.default = {
            packages = [
              toolchain
              pkgs.gnumake
              pkgs.perl
              pkgs.stdenv.cc
              pkgs.pkg-config
            ];

            motd = "";
          };
        };
    };
}
