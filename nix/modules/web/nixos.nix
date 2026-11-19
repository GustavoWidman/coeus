# `coeus web` as a systemd service of its own unprivileged user, sandboxed with
# the cloudflared and tailscale CLI it runs
{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.services.coeus;
  web = cfg.web;
  tailscale = lib.optional config.services.tailscale.enable "tailscaled.service";
  user = "coeus-web";
  state = "/var/lib/coeus-web";
  fetchCert = web.signin.certFile == null;

  # credential names, each to the file it holds
  secrets = {
    "coeus-web.read-key" = cfg.readKeyFile;
    "coeus-web.session-key" = web.sessionKeyFile;
  }
  // lib.optionalAttrs (web.tunnel != null) { "coeus-web.tunnel" = web.tunnel.credentialsFile; };
  # read on every request, so the dashboard starts without them and a
  # missing list only leaves the app unable to tell which devices get pushes
  optional = lib.optionalAttrs (web.push != null) {
    "coeus-web.push-subscriptions" = web.push.subscriptionsFile;
  };

  # a credential systemd cannot find in /run/credstore is left out, where a
  # missing file named by its path fails the start, and the switch with it.
  # coeus web exiting at once would fail it too, so it waits a while first,
  # and systemd starts it again with whatever exists by then.
  waits = lib.mapAttrsToList (name: file: ''
    if [ ! -e "$CREDENTIALS_DIRECTORY/${name}" ]; then
      echo "${file} does not exist; trying again in a minute" >&2
      sleep 60
      exit 1
    fi
  '') secrets;
  start = pkgs.writeShellScript "coeus-web-start" ''
    ${lib.concatStrings waits}
    exec ${
      lib.escapeShellArgs [
        (lib.getExe' cfg.package "coeus")
        "--verbose"
        "--config"
        "/etc/coeus/web.toml"
        "web"
        "--web-dir"
        web.package
      ]
    }
  '';
in
{
  config = lib.mkIf web.enable {
    users.users.${user} = {
      isSystemUser = true;
      group = user;
      description = "coeus web";
    };
    users.groups.${user} = { };

    systemd.tmpfiles.rules = lib.mapAttrsToList (
      name: file: "L+ /run/credstore/${name} - - - - ${file}"
    ) (secrets // optional);

    systemd.services.coeus-web = {
      description = "coeus web, the fleet's dashboard";
      wantedBy = [ "multi-user.target" ];
      wants = [
        "network-online.target"
        "coeusd.service"
      ]
      ++ tailscale
      ++ lib.optional fetchCert "coeus-web-cert.service";
      after = [
        "network-online.target"
        "coeusd.service"
      ]
      ++ tailscale;
      restartTriggers = [ config.environment.etc."coeus/web.toml".source ];
      path = lib.optional (web.tailscalePackage != null) web.tailscalePackage;

      # a missing secret keeps it retrying for as long as it takes
      startLimitIntervalSec = 0;

      serviceConfig = {
        ExecStart = start;
        Restart = "always";
        RestartSec = 5;

        User = user;
        Group = user;
        LoadCredential = lib.attrNames (secrets // optional);
        # the origin socket and cloudflared's config
        RuntimeDirectory = "coeus-web";
        RuntimeDirectoryMode = "0700";
        # the fetched certificate
        StateDirectory = "coeus-web";
        StateDirectoryMode = "0700";
        UMask = "0077";

        # cloudflared and the tailscale CLI run inside this sandbox too
        CapabilityBoundingSet = "";
        DevicePolicy = "closed";
        LockPersonality = true;
        MemoryDenyWriteExecute = true;
        NoNewPrivileges = true;
        PrivateDevices = true;
        PrivateTmp = true;
        PrivateUsers = true;
        ProcSubset = "pid";
        ProtectClock = true;
        ProtectControlGroups = true;
        ProtectHome = true;
        ProtectHostname = true;
        ProtectKernelLogs = true;
        ProtectKernelModules = true;
        ProtectKernelTunables = true;
        ProtectProc = "invisible";
        ProtectSystem = "strict";
        RemoveIPC = true;
        RestrictAddressFamilies = [
          "AF_INET"
          "AF_INET6"
          "AF_UNIX"
        ];
        RestrictNamespaces = true;
        RestrictRealtime = true;
        RestrictSUIDSGID = true;
        SystemCallArchitectures = "native";
        SystemCallFilter = [
          "@system-service"
          "~@privileged"
          "~@resources"
        ];
        # go raises its open file limit at start, and lives with being refused
        SystemCallErrorNumber = "EPERM";
      };
    };

    systemd.services.coeus-web-cert = lib.mkIf fetchCert {
      description = "coeus web's tailnet certificate from tailscaled";
      wants = [ "network-online.target" ] ++ tailscale;
      after = [
        "network-online.target"
      ]
      ++ tailscale;
      path = [ "/run/current-system/sw" ];
      serviceConfig = {
        Type = "oneshot";
        ExecStart = import ./tailscale-cert.nix {
          inherit lib pkgs;
          tailscale = if web.tailscalePackage == null then "tailscale" else lib.getExe web.tailscalePackage;
          owner = user;
          dir = state;
        };
      };
    };

    systemd.timers.coeus-web-cert = lib.mkIf fetchCert {
      wantedBy = [ "timers.target" ];
      timerConfig = {
        OnCalendar = "hourly";
        Persistent = true;
      };
    };

    networking.firewall.interfaces.${config.services.tailscale.interfaceName}.allowedTCPPorts = [
      web.signin.port
    ];
  };
}
