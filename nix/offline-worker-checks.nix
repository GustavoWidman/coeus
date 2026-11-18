{
  pkgs,
  self,
  module,
  servers,
}:
let
  inherit (pkgs) lib runCommandLocal bun;
  # a plain file with no derivation behind it, so every system evaluates the same path
  context = builtins.attrValues (builtins.getContext module);
  rejects = hosts: !(builtins.tryEval (self.lib.offlineWorker { inherit hosts; })).success;
in
{
  # the Worker's branches and the page's policy, run against the built module
  test =
    runCommandLocal "coeus-offline-worker-test"
      {
        nativeBuildInputs = [ bun ];
        OFFLINE_WORKER = module;
        OFFLINE_WORKER_HOSTS = toString servers;
      }
      ''
        export HOME=$TMPDIR
        cp ${../worker/worker.test.ts} worker.test.ts
        bun test ./worker.test.ts
        touch $out
      '';

  portable =
    assert lib.assertMsg (
      context == [ { path = true; } ]
    ) "offlineWorker depends on a derivation, so its store path differs between systems";
    assert lib.assertMsg (
      rejects [ "<b>" ] && rejects [ "-lab" ] && rejects [ "" ]
    ) "offlineWorker takes a host name that is not one";
    runCommandLocal "coeus-offline-worker-portable" { } "touch $out";
}
