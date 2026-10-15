{
  config,
  options,
  lib,
  ...
}:
let
  cfg = config.services.coeusd;
  enabled = cfg.clientConfigFile != null;
  clientModule =
    {
      config,
      lib,
      pkgs,
      ...
    }:
    {
      home.activation.coeusClientConfig = lib.hm.dag.entryAfter [ "writeBoundary" ] ''
        source_config=${lib.escapeShellArg cfg.clientConfigFile}
        config_home=${lib.escapeShellArg config.xdg.configHome}
        coreutils=${lib.escapeShellArg "${pkgs.coreutils}/bin"}
        ${builtins.readFile ./client-config.sh}
        unset source_config config_home coreutils
      '';
    };
in
{
  config = lib.mkMerge [
    {
      assertions = lib.mkIf enabled [
        {
          assertion =
            lib.hasPrefix "/" cfg.clientConfigFile
            && cfg.clientConfigFile != "/nix/store"
            && !(lib.hasPrefix "/nix/store/" cfg.clientConfigFile);
          message = "services.coeusd.clientConfigFile must be an absolute runtime path outside /nix/store";
        }
      ];
    }
    # Do not declare any HM options on hosts which have not imported Home Manager.
    (lib.optionalAttrs (options ? home-manager.users) {
      home-manager.sharedModules = lib.mkIf enabled [ clientModule ];
    })
  ];
}
