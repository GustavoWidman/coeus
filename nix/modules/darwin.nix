{ config, lib, ... }:
let
  cfg = config.services.coeus;
in
{
  options.services.coeus.launcher = lib.mkOption {
    type = lib.types.str;
    default = "/usr/bin/time";
    description = ''
      The Apple-signed program coeus' deployment runners start under. Grant it
      Full Disk Access once so nix-darwin's activation passes its checks.
    '';
  };

  config = lib.mkIf cfg.enable {
    services.coeus.settings.launcher = cfg.launcher;

    launchd.daemons.coeusd = {
      command = "/run/current-system/sw/bin/coeusd --config /etc/coeus/coeusd.toml";

      serviceConfig = {
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
