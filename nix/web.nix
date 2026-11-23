{
  lib,
  bun2nix,
  version,
}:
let
  sources = [
    ../web/package.json
    ../web/bun.lock
    ../web/.npmrc
    ../web/tsconfig.json
    ../web/vite.config.ts
    ../web/index.html
    ../web/scripts/compress.ts
    ../web/scripts/service-worker.ts
    ../web/src
    ../web/static
    # the offline page, which the service worker keeps a copy of
    ../worker/offline.html
    ../worker/offline.css
    ../worker/offline.js
  ];

  # the web sources sit beside the rest of the repository, and every build runs in web/
  source =
    extra:
    lib.fileset.toSource {
      root = ../.;
      fileset = lib.fileset.unions (sources ++ extra);
    };

  common = {
    inherit version;
    bunDeps = bun2nix.fetchBunDeps { bunNix = ../web/.bun.nix; };
    sourceRoot = "source/web";
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
        # the stream's parser, which reads every message of contract/web.json, the service
        # worker's reading of the push in contract/push.json, and what the screens derive
        # from the fleet
        unit = bun2nix.mkDerivation (
          common
          // {
            pname = "coeus-web-unit";
            src = source [
              ../contract/web.json
              ../contract/push.json
            ];
            buildPhase = ''
              runHook preBuild
              bun --bun run test
              runHook postBuild
            '';
            installPhase = "touch $out";
          }
        );
        # fails when the JS or CSS that index.html loads outgrows web/perf-budget.json
        budget = check "budget" [ ../web/scripts/budget.ts ../web/perf-budget.json ] ''
          bun --bun run build
          bun --bun scripts/budget.ts
        '';
        # fails when the built CSS animates anything but transform, opacity or clip-path
        motion = check "motion" [ ../web/scripts/motion-lint.ts ] ''
          bun --bun scripts/motion-lint.ts ${web}
        '';
        # types and formatting
        check =
          check "check"
            [
              ../web/scripts
              ../web/tests
              ../web/matrix-budget.json
              ../web/playwright.config.ts
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
