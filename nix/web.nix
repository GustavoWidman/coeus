{
  lib,
  bun2nix,
  version,
}:
let
  root = ../web;

  sources = [
    ../web/package.json
    ../web/bun.lock
    ../web/.npmrc
    ../web/tsconfig.json
    ../web/vite.config.ts
    ../web/index.html
    ../web/scripts/compress.ts
    ../web/src
    ../web/static
  ];

  source =
    extra:
    lib.fileset.toSource {
      inherit root;
      fileset = lib.fileset.unions (sources ++ extra);
    };

  common = {
    inherit version;
    bunDeps = bun2nix.fetchBunDeps { bunNix = ../web/.bun.nix; };
    dontFixup = true;
  };

  # a check: runs `script` in the web sources, with their dependencies installed
  check =
    name: extra: script:
    bun2nix.mkDerivation (
      common
      // {
        pname = "coeus-web-${name}";
        src = source extra;
        buildPhase = ''
          runHook preBuild
          ${script}
          runHook postBuild
        '';
        installPhase = "touch $out";
      }
    );

  # the static build, with a .br, .gz and .etag beside each file for the server
  # to send as they are
  web = bun2nix.mkDerivation (
    common
    // {
      pname = "coeus-web";
      src = source [ ];

      buildPhase = ''
        runHook preBuild
        bun --bun run build
        runHook postBuild
      '';

      installPhase = ''
        runHook preInstall
        mkdir -p $out
        cp -r build/. $out/
        runHook postInstall
      '';

      passthru.checks = {
        # fails when the JS or CSS that index.html loads outgrows web/perf-budget.json
        budget = check "budget" [ ../web/scripts/budget.ts ../web/perf-budget.json ] ''
          bun --bun run build
          bun --bun scripts/budget.ts
        '';
        # types and formatting
        check =
          check "check"
            [
              ../web/scripts
              ../web/prettier.config.js
              ../web/.prettierignore
            ]
            ''
              bun --bun run check
              bun --bun run lint
            '';
      };
    }
  );
in
web
