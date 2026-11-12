{ config, lib, ... }:
let
  cfg = config.services.coeus;
in
{
  options.services.coeus.launcher = lib.mkOption {
    type = lib.types.str;
    default = "/Library/Application Support/coeus/coeus-launcher";
    description = ''
      The program coeus' deployment runners start under. coeusd installs it
      once and never replaces it; granting it Full Disk Access lets
      nix-darwin's activation create and delete users.
    '';
  };

  config = lib.mkIf cfg.enable {
    services.coeus.settings.launcher = cfg.launcher;

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
