{
  lib,
  craneLib,
  makeWrapper,
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
    nativeBuildInputs = [ makeWrapper ];

    # the tests run once, in `passthru.checks.test`
    doCheck = false;

    # `coeus attach` renders a deployment by piping it into `nom --json`
    postInstall = ''
      wrapProgram $out/bin/coeus --prefix PATH : ${lib.makeBinPath [ nix-output-monitor ]}
    '';

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
