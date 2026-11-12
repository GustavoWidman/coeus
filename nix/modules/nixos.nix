{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.services.coeus;
  port = lib.toInt (lib.last (lib.splitString ":" cfg.listen));
  tailscale = lib.optional config.services.tailscale.enable "tailscaled.service";
in
{
  options.services.coeus.openFirewall = lib.mkOption {
    type = lib.types.bool;
    default = false;
    description = "Open coeusd's port on the `tailscale0` interface.";
  };

  config = lib.mkIf cfg.enable {
    systemd.services.coeusd = {
      description = "coeus deployment daemon";
      wantedBy = [ "multi-user.target" ];
      wants = [ "network-online.target" ] ++ tailscale;
      after = [ "network-online.target" ] ++ tailscale;

      # a switch must never restart coeusd under the deployment it runs;
      # coeusd restarts itself when idle after its binary or config changed
      restartIfChanged = false;
      stopIfChanged = false;

      path = [
        config.nix.package
        pkgs.git
        pkgs.openssh
      ];

      # root and unsandboxed: its runners build and activate whole systems
      serviceConfig = {
        ExecStart = "/run/current-system/sw/bin/coeusd --config /etc/coeus/coeusd.toml";
        Restart = "always";
        RestartSec = 2;
        StateDirectory = "coeus";
        StateDirectoryMode = "0700";
      };
    };

    networking.firewall.interfaces.tailscale0.allowedTCPPorts = lib.mkIf cfg.openFirewall [ port ];
  };
}
