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
    nix-darwin = {
      url = "github:nix-darwin/nix-darwin";
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
        web = flake-parts-lib.importApply ./nix/modules/web/common.nix { inherit self; };

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
            web
            ./nix/modules/web/nixos.nix
          ];
          darwinModules.default.imports = [
            common
            ./nix/modules/darwin.nix
            web
            ./nix/modules/web/darwin.nix
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
              inherit coeus;
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
            }
            # every cell of the state matrix, screenshots included, where its check runs
            // lib.optionalAttrs (system == "x86_64-linux") {
              coeus-web-matrix = coeus-web.matrix;
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
              web = pkgs.testers.runNixOSTest (import ./nix/web-test.nix { inherit self; });
            }
            // lib.optionalAttrs pkgs.stdenv.hostPlatform.isDarwin {
              darwin-module = import ./nix/darwin-check.nix {
                inherit pkgs self system;
                inherit (inputs) nix-darwin;
              };
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
                # the browsers of the Playwright release web/package.json pins
                {
                  name = "PLAYWRIGHT_BROWSERS_PATH";
                  value = "${pkgs.playwright-driver.browsers}";
                }
                {
                  name = "PLAYWRIGHT_VERSION";
                  value = pkgs.playwright-driver.version;
                }
                {
                  name = "PLAYWRIGHT_SKIP_VALIDATE_HOST_REQUIREMENTS";
                  value = "1";
                }
              ];

              commands = [
                {
                  name = "matrix";
                  help = "build coeus and the web app, then run web/scripts/matrix.ts with the given flags";
                  command = ''
                    set -eu
                    cd "$PRJ_ROOT"
                    cargo build --release
                    cd web
                    bun install --frozen-lockfile
                    bun run build
                    bun scripts/matrix.ts "$@"
                  '';
                }
                {
                  name = "e2e";
                  help = "build coeus and the web app, then run the end-to-end tests on a phone and a desktop";
                  command = ''
                    set -eu
                    cd "$PRJ_ROOT"
                    cargo build --release
                    cd web
                    bun install --frozen-lockfile
                    bun run build
                    bunx playwright test "$@"
                  '';
                }
                {
                  name = "matrix-bless";
                  help = "run the state matrix on x86_64-linux and write its screenshots' hashes as the check's baselines";
                  command = ''
                    set -eu
                    cd "$PRJ_ROOT/web"
                    bun install --frozen-lockfile
                    out=$(nix build "$PRJ_ROOT#packages.x86_64-linux.coeus-web-matrix" --no-link --print-out-paths)
                    echo "screenshots: $out/shots"
                    bun scripts/matrix.ts --from "$out/matrix-results.json" --no-timings --update
                  '';
                }
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
