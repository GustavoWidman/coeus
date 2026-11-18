# `coeus web` as a launchd daemon of its own hidden user, which owns its
# directory and must be able to read the files it is given
{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.services.coeus;
  web = cfg.web;
  user = "_coeusweb";
  state = "/var/lib/coeus-web";
  log = "/var/log/coeus-web.log";
  path = lib.concatStringsSep ":" (
    lib.optional (web.tailscalePackage != null) "${web.tailscalePackage}/bin"
    ++ [
      "/run/current-system/sw/bin"
      "/usr/bin"
      "/bin"
      "/usr/sbin"
      "/sbin"
    ]
  );
in
{
  options.services.coeus.web.uid = lib.mkOption {
    type = lib.types.int;
    default = 471;
    description = ''
      The uid, and gid, of `${user}`, the user coeus web runs as. Give it
      the read-only key, the session key, the tunnel's credentials and a
      given certificate (agenix's `owner`).
    '';
  };

  config = lib.mkIf web.enable {
    users.knownUsers = [ user ];
    users.knownGroups = [ user ];
    users.users.${user} = {
      inherit (web) uid;
      gid = web.uid;
      home = state;
      shell = "/usr/bin/false";
      isHidden = true;
      description = "coeus web";
    };
    users.groups.${user} = {
      gid = web.uid;
      description = "coeus web";
    };

    # the origin socket, cloudflared's config and the fetched certificate,
    # and a log the daemon's user may write
    system.activationScripts.postActivation.text = ''
      install -d -m 0700 -o ${user} -g ${user} ${state}
      touch ${log}
      chown ${user}:${user} ${log}
    '';

    launchd.daemons.coeus-web = {
      command = lib.escapeShellArgs [
        (lib.getExe' cfg.package "coeus")
        "--verbose"
        "--config"
        "/etc/coeus/web.toml"
        "web"
        "--web-dir"
        web.package
      ];
      serviceConfig = {
        UserName = user;
        GroupName = user;
        KeepAlive = true;
        RunAtLoad = true;
        # cloudflared stays in the process group, so it dies with coeus web
        AbandonProcessGroup = false;
        StandardOutPath = log;
        StandardErrorPath = log;
        EnvironmentVariables.PATH = path;
      };
    };

    launchd.daemons.coeus-web-cert = lib.mkIf (web.signin.certFile == null) {
      command = "${import ./tailscale-cert.nix {
        inherit lib pkgs;
        tailscale =
          if web.tailscalePackage == null then
            "$(command -v tailscale || echo /Applications/Tailscale.app/Contents/MacOS/Tailscale)"
          else
            lib.getExe web.tailscalePackage;
        owner = user;
        dir = state;
      }}";
      serviceConfig = {
        RunAtLoad = true;
        StartInterval = 3600;
        StandardOutPath = log;
        StandardErrorPath = log;
        EnvironmentVariables.PATH = path;
      };
    };
  };
}
