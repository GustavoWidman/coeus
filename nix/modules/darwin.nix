{ config, lib, ... }:
let
  cfg = config.services.coeus;
  launcher = lib.escapeShellArg cfg.launcher;
  activation = config.launchd.daemons.activate-system.command;
in
{
  options.services.coeus.launcher = lib.mkOption {
    type = lib.types.str;
    default = "/Library/Application Support/coeus/coeus-launcher";
    description = ''
      The program coeus' deployment runners and nix-darwin's activation at
      boot start under. coeusd installs it once and never replaces it;
      granting it Full Disk Access lets nix-darwin's activation create and
      delete users, and update the apps in /Applications/Nix Apps.
    '';
  };

  config = lib.mkIf cfg.enable {
    services.coeus.settings.launcher = cfg.launcher;

    # nix-darwin activates the system again at every boot, from its
    # activate-system daemon, and that run first checks that it may write
    # inside each bundle in /Applications/Nix Apps. macos allows that only to
    # a program granted App Management or Full Disk Access, and holds the
    # daemon's own program responsible: the nix store's bash, whose path
    # changes with every build, so no grant stays on it. the check fails and
    # activation stops short of /etc and the keyboard mapping. under the
    # launcher, the boot activation has the grant every deployment's has;
    # until coeusd installs the launcher, it starts as nix-darwin starts it
    launchd.daemons.activate-system.serviceConfig.ProgramArguments = lib.mkForce [
      "/bin/sh"
      "-c"
      "/bin/wait4path /nix/store && if [ -x ${launcher} ]; then exec ${launcher} ${activation}; fi; exec ${activation}"
    ];

    launchd.daemons.coeusd = {
      serviceConfig = {
        # activation repoints /run/current-system after loading daemons, so
        # the first start after an install waits for the binary to appear
        ProgramArguments = [
          "/bin/sh"
          "-c"
          "/bin/wait4path /run/current-system/sw/bin/coeusd && exec /run/current-system/sw/bin/coeusd --config /etc/coeus/coeusd.toml"
        ];
        KeepAlive = true;
        RunAtLoad = true;
        ProcessType = "Standard";
        # a coeusd exit must not kill the processes it started
        AbandonProcessGroup = true;
        StandardOutPath = "/var/log/coeusd.log";
        StandardErrorPath = "/var/log/coeusd.log";
        EnvironmentVariables = {
          PATH = "/run/current-system/sw/bin:/nix/var/nix/profiles/default/bin:/usr/bin:/bin:/usr/sbin:/sbin";
          HOME = "/var/root";
          inherit (config.environment.variables) NIX_SSL_CERT_FILE;
        };
      };
    };
  };
}
