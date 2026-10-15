self:
{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.services.coeusd;
  runtime = import ./runtime.nix {
    inherit cfg pkgs lib;
    configHost = config.networking.hostName;
    platform = "linux";
    accountUid = config.users.users.${cfg.user}.uid or null;
    accountGid = config.users.groups.${cfg.group}.gid or null;
  };
in
{
  imports = [ (import ./options.nix self) ];
  config = lib.mkIf cfg.enable {
    system.systemBuilderCommands = ''
      cp ${runtime.manifest} $out/coeus-service.toml
    '';
    assertions = runtime.assertions;
    users.users = lib.mkIf cfg.createUser {
      ${cfg.user} = {
        isSystemUser = true;
        group = cfg.group;
        home = cfg.home;
        createHome = true;
      };
    };
    users.groups = lib.mkIf cfg.createUser { ${cfg.group} = { }; };
    systemd.tmpfiles.rules = [ "d ${cfg.stateDirectory} 0700 ${cfg.user} ${cfg.group} -" ];
    systemd.services.coeusd = {
      description = "Coeus deployment daemon";
      wantedBy = [ "multi-user.target" ];
      after = [ "network-online.target" ];
      wants = [ "network-online.target" ];
      # Do not stop an in-flight self-switch solely because its definition changes.
      restartIfChanged = false;
      reloadIfChanged = false;
      stopIfChanged = false;
      environment = runtime.environment // {
        PATH = lib.mkForce runtime.path;
      };
      serviceConfig = {
        ExecStart = runtime.runner;
        User = cfg.user;
        Group = cfg.group;
        WorkingDirectory = cfg.stateDirectory;
        UMask = "0077";
        Restart = "on-failure";
        RestartSec = 5;
        # NoNewPrivileges/RestrictSUIDSGID would prevent the explicit sudo boundary.
      };
    };
    security.sudo = lib.mkIf cfg.enablePasswordlessActivation {
      enable = true;
      extraRules = [
        {
          users = [ cfg.user ];
          commands = [
            {
              command = toString runtime.helper;
              options = [ "NOPASSWD" ];
            }
          ];
        }
      ];
    };
  };
}
