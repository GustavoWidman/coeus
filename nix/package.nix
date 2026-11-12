{
  lib,
  craneLib,
  nix-output-monitor,
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
      test = craneLib.cargoTest (args // { inherit cargoArtifacts; });
      clippy = craneLib.cargoClippy (
        args
        // {
          inherit cargoArtifacts;
          cargoClippyExtraArgs = "--all-targets";
        }
      );
    };

    meta.mainProgram = "coeus";
  }
)
