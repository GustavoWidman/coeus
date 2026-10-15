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
    platform = "darwin";
    accountUid = cfg.uid;
    accountGid = cfg.gid;
  };
in
{
  imports = [ (import ./options.nix self) ];
  options.services.coeusd.uid = lib.mkOption {
    type = lib.types.nullOr lib.types.int;
    default = null;
    description = "Unused host UID required when creating the dedicated Darwin service account (no guessed UID).";
  };
  options.services.coeusd.gid = lib.mkOption {
    type = lib.types.int;
    default = 20;
    description = "Existing primary group ID for a created Darwin account; 20 is staff. Match this with group when overriding.";
  };
  config = lib.mkIf cfg.enable {
    system.systemBuilderCommands = ''
      cp ${runtime.manifest} $out/coeus-service.toml
    '';
    assertions = runtime.assertions ++ [
      {
        assertion = !cfg.createUser || (cfg.uid != null && cfg.uid > 0 && cfg.gid >= 0);
        message = "services.coeusd.uid must be explicitly allocated when creating a Darwin account";
      }
    ];
    users.knownUsers = lib.mkIf cfg.createUser [ cfg.user ];
    users.users = lib.mkIf cfg.createUser {
      ${cfg.user} = {
        uid = cfg.uid;
        gid = cfg.gid;
        home = cfg.home;
        createHome = true;
        isHidden = true;
        shell = "/usr/bin/false";
      };
    };
    # nix-darwin creates users before this fragment, then loads launchd jobs.
    # postActivation is too late for WorkingDirectory and launchd log paths.
    system.activationScripts.launchd.text = lib.mkBefore ''
      ${pkgs.coreutils}/bin/install -d -m 0700 -o ${lib.escapeShellArg cfg.user} -g ${lib.escapeShellArg cfg.group} ${lib.escapeShellArg cfg.stateDirectory}
    '';
    launchd.daemons.coeusd = {
      serviceConfig = {
        ProgramArguments = [ (toString runtime.runner) ];
        UserName = cfg.user;
        GroupName = cfg.group;
        WorkingDirectory = cfg.stateDirectory;
        StandardOutPath = "${cfg.stateDirectory}/coeusd.stdout.log";
        StandardErrorPath = "${cfg.stateDirectory}/coeusd.stderr.log";
        EnvironmentVariables = runtime.environment;
        RunAtLoad = true;
        KeepAlive = {
          SuccessfulExit = false;
        };
        ThrottleInterval = 5;
        Umask = 63;
      };
    };
    security.sudo.extraConfig = lib.mkIf cfg.enablePasswordlessActivation ''
      ${cfg.user} ALL=(root) NOPASSWD: ${runtime.helper}
    '';
  };
}
