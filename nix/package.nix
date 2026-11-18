{
  lib,
  craneLib,
  nix-output-monitor,
  git,
  nix,
  ps,
  stdenv,
}:
let
  root = ../.;

  # the probe a bootstrap runs on a stock machine is compiled into the binary
  sources = lib.fileset.unions [
    (craneLib.fileset.commonCargoSources root)
    ../src/bootstrap/remote/probe.sh
  ];

  args = {
    src = lib.fileset.toSource {
      inherit root;
      fileset = sources;
    };
    strictDeps = true;
  };

  cargoVendorDir = craneLib.vendorCargoDeps args;

  # the dependencies build against coeus and its workspace's crates without
  # their versions, so a release, which bumps only those, keeps the dependency
  # build CI cached for the last
  locked = builtins.readFile ../Cargo.lock;
  # the packages the lock names no source for: the workspace's own
  workspace = builtins.filter (package: !(package ? source)) (builtins.fromTOML locked).package;
  entry = name: version: ''
    name = "${name}"
    version = "${version}"
  '';
  unversionedLock =
    let
      versioned = map (package: entry package.name package.version) workspace;
      unversioned = map (package: entry package.name "0.0.0") workspace;
    in
    # found by replacing it, not with lib.hasInfix, whose regex over the whole
    # lock libstdc++ runs one stack frame a character: past a few thousand
    # lines it overflows an evaluator's stack on linux
    assert lib.assertMsg (lib.all (
      text: builtins.replaceStrings [ text ] [ "" ] locked != locked
    ) versioned) "Cargo.lock lays out a workspace package differently than expected";
    builtins.toFile "Cargo.lock" (builtins.replaceStrings versioned unversioned locked);

  # its name, version and vendored sources are given, so crane never reads
  # this generated source while evaluating
  cargoArtifacts = craneLib.buildDepsOnly (
    removeAttrs args [ "src" ]
    // {
      inherit cargoVendorDir;
      pname = "coeus";
      version = "0.0.0";
      dummySrc = craneLib.mkDummySrc (
        args
        // {
          src = lib.fileset.toSource {
            inherit root;
            fileset = lib.fileset.difference sources ../Cargo.lock;
          };
          cargoLock = unversionedLock;
          cleanCargoTomlFilter =
            path:
            craneLib.filters.cargoTomlDefault path
            &&
              path != [
                "package"
                "version"
              ];
        }
      );
    }
  );

  # the tests also read recorded nix output, sample contracts and the web
  # dashboard's contract and types, which the package itself never needs
  checkArgs = args // {
    inherit cargoArtifacts;
    src = lib.fileset.toSource {
      inherit root;
      fileset = lib.fileset.unions [
        sources
        ../src/nix/fixtures
        ../src/bootstrap/contract/test/fixtures
        ../src/bootstrap/test/run/contract.nix
        ../contract/web.json
        ../web/src/lib/wire/types.ts
      ];
    };
  };
in
craneLib.buildPackage (
  args
  // {
    inherit cargoArtifacts;

    # the tests run once, in `passthru.checks.test`
    doCheck = false;

    # coeusd draws attaches with nom, on the machine whose store holds the
    # derivations it reads
    COEUS_NOM = lib.getExe nix-output-monitor;

    passthru.checks = {
      # snapshots go through git and nix, and the runner tests look processes up
      test = craneLib.cargoTest (
        checkArgs
        // {
          nativeBuildInputs = [
            git
            nix
            ps
          ];
          preConfigure = ''
            export HOME="$TMPDIR/home"
            mkdir -p "$HOME"
          '';
          # the kill tests rerun the bootstrap from every point it can stop at, each
          # starting thousands of git and nix processes, and a macOS runner starts a
          # process about fifteen times as slowly as a Linux one. the bootstrap has no
          # platform-specific code, so they run on Linux alone.
          cargoTestExtraArgs = lib.optionalString stdenv.hostPlatform.isDarwin "-- --skip bootstrap::test::run::kill::";
        }
      );
      clippy = craneLib.cargoClippy (checkArgs // { cargoClippyExtraArgs = "--all-targets"; });
    };

    meta.mainProgram = "coeus";
  }
)
