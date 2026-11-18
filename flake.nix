{
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
    crane.url = "github:ipetkov/crane";
    flake-parts.url = "github:hercules-ci/flake-parts";
    devshell.url = "github:numtide/devshell";
    bun2nix = {
      url = "github:baileyluTCD/bun2nix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    inputs@{
      flake-parts,
      crane,
      devshell,
      bun2nix,
      ...
    }:
    flake-parts.lib.mkFlake { inherit inputs; } (
      { self, flake-parts-lib, ... }:
      let
        common = flake-parts-lib.importApply ./nix/modules/common.nix { inherit self; };

        # the fleet's serving hosts, for the offline Worker that `nix build` and the checks use
        servers = [
          "altar"
          "lab"
          "oracle-1"
          "oracle-2"
        ];
        offlineWorker = self.lib.offlineWorker { hosts = servers; };
      in
      {
        # nixpkgs no longer evaluates for x86_64-darwin
        systems = [
          "aarch64-darwin"
          "aarch64-linux"
          "x86_64-linux"
        ];
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

          # `{ hosts }` to the offline page Worker's module, the same store path on every system
          lib.offlineWorker = import ./nix/offline-worker.nix;
        };

        perSystem =
          {
            pkgs,
            system,
            lib,
            ...
          }:
          let
            # nixpkgs' stable rust, which cache.nixos.org serves ready to run, for
            # builds, checks and the devshell alike
            craneLib = crane.mkLib pkgs;
            coeus = pkgs.callPackage ./nix/package.nix { inherit craneLib; };
            bunLib = bun2nix.packages.${system}.default;
            coeus-web = pkgs.callPackage ./nix/web.nix {
              bun2nix = bunLib;
              inherit (coeus) version;
            };
          in
          {
            packages = {
              default = coeus;
              inherit coeus-web;
              offline-worker = pkgs.runCommandLocal "coeus-offline-worker" { } ''
                mkdir $out
                cp ${offlineWorker} $out/worker.js
              '';
            };

            checks = {
              inherit coeus coeus-web;
            }
            // coeus.passthru.checks
            // lib.mapAttrs' (name: lib.nameValuePair "coeus-web-${name}") coeus-web.passthru.checks
            // lib.mapAttrs' (name: lib.nameValuePair "offline-worker-${name}") (
              import ./nix/offline-worker-checks.nix {
                inherit pkgs self servers;
                module = offlineWorker;
              }
            )
            // lib.optionalAttrs pkgs.stdenv.hostPlatform.isLinux {
              nixos = pkgs.testers.runNixOSTest (import ./nix/test.nix { inherit self; });
            };

            devshells.default = {
              packages = with pkgs; [
                cargo
                rustc
                clippy
                rustfmt
                rust-analyzer
                bun
              ];
              env = [
                {
                  name = "RUST_SRC_PATH";
                  value = "${pkgs.rustPlatform.rustLibSrc}";
                }
              ];

              commands = [
                {
                  name = "lock-web";
                  help = "regenerate web/.bun.nix from web/bun.lock";
                  # bun2nix refuses bun.lock's lockfileVersion 2, although the package layout
                  # it reads is the same as in version 1
                  command = ''
                    set -eu
                    cd "$PRJ_ROOT/web"
                    lock=$(mktemp)
                    trap 'rm -f "$lock"' EXIT
                    sed 's/"lockfileVersion": 2,/"lockfileVersion": 1,/' bun.lock > "$lock"
                    ${lib.getExe bunLib} --lock-file "$lock" --output-file .bun.nix
                  '';
                }
              ];

              motd = "";
            };
          };
      }
    );
}
