{
  lib,
  craneLib,
  nix-output-monitor,
  git,
  nix,
  ps,
}:
let
  root = ../.;

  args = {
    src = lib.fileset.toSource {
      inherit root;
      fileset = craneLib.fileset.commonCargoSources root;
    };
    strictDeps = true;
  };

  cargoArtifacts = craneLib.buildDepsOnly args;

  # the tests also read recorded nix output, which the package itself never needs
  checkArgs = args // {
    inherit cargoArtifacts;
    src = lib.fileset.toSource {
      inherit root;
      fileset = lib.fileset.unions [
        (craneLib.fileset.commonCargoSources root)
        ../src/nix/fixtures
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
        }
      );
      clippy = craneLib.cargoClippy (checkArgs // { cargoClippyExtraArgs = "--all-targets"; });
    };

    meta.mainProgram = "coeus";
  }
)
