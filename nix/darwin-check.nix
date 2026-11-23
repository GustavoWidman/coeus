# evaluates a nix-darwin system serving the dashboard, and checks the daemons,
# the web.toml, coeusd.toml and the CLI's coeus.toml it renders, and how the
# boot activation starts, without building the system
{
  pkgs,
  self,
  system,
  nix-darwin,
}:
let
  darwin = nix-darwin.lib.darwinSystem {
    modules = [
      self.darwinModules.default
      {
        nixpkgs.hostPlatform = system;
        system.stateVersion = 6;
        networking.hostName = "altar";
        services.tailscale.enable = true;
        services.coeus = {
          enable = true;
          keyFile = "/run/agenix/coeus-key";
          readKeyFile = "/run/agenix/coeus-read-key";
          flake = "git+https://example.com/fleet";
          hosts.altar.tags = [ "darwin" ];
          push = {
            origin = "https://coeus.example.com";
            vapidKeyFile = "/run/agenix/coeus-push-vapid";
            subscriptionsFile = "/run/agenix/coeus-push-subscriptions";
          };
          web = {
            enable = true;
            publicOrigin = "https://coeus.example.com";
            sessionKeyFile = "/run/agenix/coeus-session-key";
            signin.urls = [ "https://altar.tail0000.ts.net:45730" ];
            tunnel = {
              id = "6f1c2a9e-3b7d-4e58-9a0c-d2e4f6a8b1c3";
              credentialsFile = "/run/agenix/coeus-tunnel";
            };
            push = {
              publicKey = "BBgqG0VbQAPnBlmSzWD_uRkfTJJ7rNkfZc3HbxlUfhd8DBhqYt_DqIxDFK71gxU01Umj-1CqpRCOP0NQP_4183g";
              subscriptionsFile = "/run/agenix/coeus-push-subscriptions-web";
              edit = "agenix -e push/subscriptions.age";
            };
          };
        };
      }
    ];
  };
  inherit (darwin) config;
  inherit (pkgs) lib;
  activate = config.launchd.daemons.activate-system;
  # the boot activation's start line, with the launcher, the activation and
  # the wait for the store swapped for stand-ins the check runs
  boot =
    lib.replaceStrings
      [
        "/bin/wait4path /nix/store"
        (lib.escapeShellArg config.services.coeus.launcher)
        (builtins.unsafeDiscardStringContext activate.command)
      ]
      [
        "true"
        "./launcher"
        "./activation"
      ]
      (builtins.unsafeDiscardStringContext (builtins.elemAt activate.serviceConfig.ProgramArguments 2));
  facts = builtins.toJSON {
    # forces the whole system, every daemon's plist and /etc included
    system = builtins.unsafeDiscardStringContext config.system.build.toplevel.drvPath;
    web = config.launchd.daemons.coeus-web.serviceConfig;
    cert = config.launchd.daemons.coeus-web-cert.serviceConfig;
    user = config.users.users._coeusweb.uid;
    activate = activate.serviceConfig.ProgramArguments;
  };
in
pkgs.runCommandLocal "coeus-darwin-module"
  {
    facts = builtins.unsafeDiscardStringContext facts;
    inherit boot;
    toml = config.environment.etc."coeus/web.toml".source;
    client = config.environment.etc."coeus/coeus.toml".source;
    daemon = config.environment.etc."coeus/coeusd.toml".source;
    nativeBuildInputs = [
      pkgs.jq
      pkgs.remarshal
    ];
  }
  ''
    jq -e '.web.UserName == "_coeusweb" and .web.KeepAlive and .web.AbandonProcessGroup == false' <<< "$facts"
    jq -e '.cert.UserName == null and .cert.StartInterval == 3600' <<< "$facts"
    jq -e '.activate[0:2] == ["/bin/sh", "-c"] and (.activate | length) == 3' <<< "$facts"
    # the boot activation starts as nix-darwin starts it until there is a
    # launcher, and under the launcher once there is one
    printf '#!/bin/sh\necho "activated''${LAUNCHED:+ under $LAUNCHED}"\n' > activation
    chmod +x activation
    [ "$(sh -c "$boot")" = activated ]
    printf '#!/bin/sh\nLAUNCHED=launcher exec "$@"\n' > launcher
    chmod +x launcher
    [ "$(sh -c "$boot")" = "activated under launcher" ]
    toml2json "$toml" > web.json
    jq -e '.settings.key_file == "/run/agenix/coeus-read-key"' web.json
    jq -e '.settings.web.origin_socket == "/var/lib/coeus-web/origin.sock"' web.json
    jq -e '.settings.web.signin.cert == "/var/lib/coeus-web/cert.pem"' web.json
    jq -e '.settings.web.tunnel.credentials_file == "/run/agenix/coeus-tunnel"' web.json
    jq -e '.settings.web.push == {
      public_key: "BBgqG0VbQAPnBlmSzWD_uRkfTJJ7rNkfZc3HbxlUfhd8DBhqYt_DqIxDFK71gxU01Umj-1CqpRCOP0NQP_4183g",
      subscriptions_file: "/run/agenix/coeus-push-subscriptions-web",
      edit: "agenix -e push/subscriptions.age"
    }' web.json
    toml2json "$client" > client.json
    jq -e '.settings.push == {
      origin: "https://coeus.example.com",
      vapid_key_file: "/run/agenix/coeus-push-vapid",
      subscriptions_file: "/run/agenix/coeus-push-subscriptions"
    }' client.json
    toml2json "$daemon" > daemon.json
    jq -e --slurpfile client client.json '.settings.push == $client[0].settings.push' daemon.json
    touch $out
  ''
