{
  lib,
  stdenv,
  bun2nix,
  version,
  coeus,
  playwright-driver,
  makeFontsConf,
  dejavu_fonts,
  mesa,
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

      # the state matrix's run: results and screenshots of every cell. The baselines and the
      # budget stay out of its sources, so changing them only reruns the check below.
      passthru.matrix = bun2nix.mkDerivation (
        common
        // {
          pname = "coeus-web-matrix";
          src = source [
            ../web/scripts
            (lib.fileset.difference ../web/tests (lib.fileset.maybeMissing ../web/tests/matrix/baselines))
          ];
          env = {
            PLAYWRIGHT_BROWSERS_PATH = playwright-driver.browsers;
            PLAYWRIGHT_VERSION = playwright-driver.version;
            # the browsers' libraries come from nixpkgs, so playwright's check
            # for the host distribution's packages means nothing here
            PLAYWRIGHT_SKIP_VALIDATE_HOST_REQUIREMENTS = "1";
            FONTCONFIG_FILE = makeFontsConf { fontDirectories = [ dejavu_fonts ]; };
          }
          # WebKit draws through EGL, which the sandbox has no driver for but mesa's software one
          // lib.optionalAttrs stdenv.hostPlatform.isLinux {
            __EGL_VENDOR_LIBRARY_DIRS = "${mesa}/share/glvnd/egl_vendor.d";
            LIBGL_DRIVERS_PATH = "${mesa}/lib/dri";
            GBM_BACKENDS_PATH = "${mesa}/lib/gbm";
            LIBGL_ALWAYS_SOFTWARE = "1";
          };
          buildPhase = ''
            runHook preBuild
            export HOME=$TMPDIR
            bun --bun scripts/matrix.ts --web-dir ${web} --coeus ${lib.getExe coeus} \
              --out $out --workers 4 --no-gate
            runHook postBuild
          '';
          installPhase = "true";
        }
      );

      passthru.checks = {
        # the stream's parser, which reads every message of contract/web.json, and what the
        # screens derive from the fleet
        unit = bun2nix.mkDerivation (
          common
          // {
            pname = "coeus-web-unit";
            src = lib.fileset.toSource {
              root = ../.;
              fileset = lib.fileset.unions (sources ++ [ ../contract/web.json ]);
            };
            sourceRoot = "source/web";
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
              ../web/prettier.config.js
              ../web/.prettierignore
            ]
            ''
              bun --bun run check
              bun --bun run lint
            '';
      }
      # Playwright's browsers run in the build sandbox on this system. Without a GPU, frame
      # timing and GPU memory measure the CPU's raster, so only the counts are checked.
      // lib.optionalAttrs (stdenv.hostPlatform.system == "x86_64-linux") {
        matrix = check "matrix" [ ../web/scripts ../web/tests ../web/matrix-budget.json ] ''
          bun --bun scripts/matrix.ts --from ${web.matrix}/matrix-results.json --no-timings
        '';
      };
    }
  );
in
web
