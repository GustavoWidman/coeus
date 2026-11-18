# `coeus web`'s options and the /etc/coeus/web.toml they render, the same on
# NixOS and nix-darwin; ./nixos.nix and ./darwin.nix run it
{ self }:
{
  config,
  lib,
  pkgs,
  ...
}:
let
  inherit (lib) mkOption types;

  cfg = config.services.coeus;
  toml = pkgs.formats.toml { };
  render = settings: lib.filterAttrsRecursive (_: value: value != null) { inherit settings; };

  web = cfg.web;
  isDarwin = pkgs.stdenv.hostPlatform.isDarwin;
  # systemd hands the service its secrets as credentials; launchd reads them where they are
  secret =
    name: file: if isDarwin then file else "/run/credentials/coeus-web.service/coeus-web.${name}";
  state = "/var/lib/coeus-web";
  # /var/run on a Mac empties at boot, and only root could make the directory again
  runtime = if isDarwin then state else "/run/coeus-web";

  webConfig = render (
    lib.recursiveUpdate {
      key_file = secret "read-key" cfg.readKeyFile;
      inherit (cfg) hosts;
      web = {
        public_origin = web.publicOrigin;
        origin_socket = "${runtime}/origin.sock";
        session_key_file = secret "session-key" web.sessionKeyFile;
        signin = {
          inherit (web.signin)
            port
            urls
            capability
            logins
            ;
          cert = if web.signin.certFile == null then "${state}/cert.pem" else web.signin.certFile;
          key = if web.signin.keyFile == null then "${state}/key.pem" else web.signin.keyFile;
        };
        tunnel = lib.mapNullable (tunnel: {
          inherit (tunnel) id;
          credentials_file = secret "tunnel" tunnel.credentialsFile;
          cloudflared = lib.getExe tunnel.package;
        }) web.tunnel;
      };
    } web.settings
  );
in
{
  options.services.coeus.web = {
    enable = mkOption {
      type = types.bool;
      default = false;
      description = ''
        Whether to run `coeus web`, the fleet's dashboard, on this machine.
        It watches every machine in `hosts` with `readKeyFile`'s key, signs
        browsers in on the tailnet, and connects to the cloudflare tunnel
        while this machine can serve. Enable it on every machine that
        serves the dashboard: any one of them up keeps the dashboard up.
      '';
    };

    package = mkOption {
      type = types.package;
      default = self.packages.${pkgs.stdenv.hostPlatform.system}.coeus-web;
      defaultText = lib.literalExpression "coeus.packages.\${system}.coeus-web";
      description = "The dashboard's built app. coeus itself comes from `services.coeus.package`.";
    };

    publicOrigin = mkOption {
      type = types.str;
      example = "https://coeus.example.com";
      description = "Where browsers load the dashboard from: the tunnel's public hostname, with its scheme.";
    };

    sessionKeyFile = mkOption {
      type = types.str;
      example = "/run/agenix/coeus-session-key";
      description = ''
        File holding the fleet's session keys, 64 hex digits a line, the
        same file on every serving machine so a session from one works on
        all of them. The first key signs new sessions and every key checks
        them: rotate by putting a new key first and dropping the old one a
        week later, or at once to end every session. `coeus keygen` makes a
        key.
      '';
    };

    signin = {
      port = mkOption {
        type = types.port;
        default = 45730;
        description = "The port signing in is served on, on this machine's tailnet addresses alone.";
      };

      urls = mkOption {
        type = types.listOf types.str;
        example = [
          "https://lab.tail0000.ts.net:45730"
          "https://oracle.tail0000.ts.net:45730"
        ];
        description = ''
          Every serving machine's sign-in address,
          `https://<name>.<tailnet>.ts.net:<port>`. The app tries them in
          turn, so list the same ones on every serving machine.
        '';
      };

      capability = mkOption {
        type = types.nullOr types.str;
        default = null;
        example = "example.com/cap/coeus";
        description = ''
          The app capability a tailnet policy grant gives the nodes that may
          sign in, toward the serving machines. coeus' own default,
          `r3dlust.com/cap/coeus`, when null.
        '';
      };

      logins = mkOption {
        type = types.listOf types.str;
        default = [ ];
        example = [ "you@example.com" ];
        description = "Tailnet logins whose own nodes may sign in without the policy grant.";
      };

      certFile = mkOption {
        type = types.nullOr types.str;
        default = null;
        example = "/var/lib/tailscale-cert/cert.pem";
        description = ''
          This machine's `*.ts.net` certificate, with its chain, as PEM. When
          null, coeus asks tailscaled for one (`tailscale cert`, which needs
          HTTPS enabled for the tailnet) into its state directory, at start
          and every hour after, so it renews itself. A file given here must
          be readable by the service's user, and coeus web picks up a
          renewed one without restarting.
        '';
      };

      keyFile = mkOption {
        type = types.nullOr types.str;
        default = null;
        example = "/var/lib/tailscale-cert/key.pem";
        description = "The private key of `certFile`, as PEM; fetched with it when null.";
      };
    };

    tunnel = mkOption {
      default = null;
      description = ''
        The cloudflare tunnel this machine connects to while it can serve:
        tailscale running and in touch with its coordination server and
        relays, and this machine's coeusd answering the read-only key.
        cloudflared runs as coeus web's child, and nothing reaches the
        dashboard when null. Give every serving machine the same tunnel.
      '';
      type = types.nullOr (
        types.submodule {
          options = {
            id = mkOption {
              type = types.str;
              example = "6f1c2a9e-3b7d-4e58-9a0c-d2e4f6a8b1c3";
              description = "The tunnel's id, from `cloudflared tunnel create`.";
            };
            credentialsFile = mkOption {
              type = types.str;
              example = "/run/agenix/coeus-tunnel";
              description = "The credentials JSON `cloudflared tunnel create` wrote for the tunnel.";
            };
            package = lib.mkPackageOption pkgs "cloudflared" { };
          };
        }
      );
    };

    tailscalePackage = mkOption {
      type = types.nullOr types.package;
      default = if config.services.tailscale.enable then config.services.tailscale.package else null;
      defaultText = lib.literalExpression "if config.services.tailscale.enable then config.services.tailscale.package else null";
      description = ''
        The tailscale CLI coeus web asks who is calling, whether this machine
        is on the tailnet, and for its certificate. When null, the
        `tailscale` on the system's PATH, or the Mac app's.
      '';
    };

    settings = mkOption {
      inherit (toml) type;
      default = { };
      example = {
        web.readiness = {
          interval = "2s";
          up_after = 3;
          down_after = 5;
        };
      };
      description = ''
        Extra `[settings]` for `/etc/coeus/web.toml`, merged over the options
        above; `web.example.toml` in coeus lists every key.
      '';
    };
  };

  config = lib.mkIf web.enable {
    assertions = [
      {
        assertion = cfg.enable;
        message = "services.coeus.web needs services.coeus: its readiness asks this machine's coeusd.";
      }
      {
        assertion = cfg.readKeyFile != null;
        message = "services.coeus.web watches the fleet with the read-only key: set services.coeus.readKeyFile.";
      }
      {
        assertion = (web.signin.certFile == null) == (web.signin.keyFile == null);
        message = "services.coeus.web.signin takes certFile and keyFile together, or neither to fetch them.";
      }
      {
        assertion = cfg.hosts != { };
        message = "services.coeus.web shows the machines in services.coeus.hosts, and there are none.";
      }
    ];

    environment.etc."coeus/web.toml".source = toml.generate "web.toml" webConfig;
  };
}
