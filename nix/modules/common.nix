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

  # option names match the TOML keys; null means "leave it to coeus' default"
  render = settings: lib.filterAttrsRecursive (_: value: value != null) { inherit settings; };

  daemon = render (
    lib.recursiveUpdate {
      key_file = cfg.keyFile;
      ssh_key_file = cfg.sshKeyFile;
      inherit (cfg)
        flake
        hostname
        listen
        history
        follow
        guard
        notify
        ;
    } cfg.settings
  );

  client = render (
    lib.recursiveUpdate {
      key_file = cfg.keyFile;
      inherit (cfg) hosts;
    } cfg.clientSettings
  );
in
{
  options.services.coeus = {
    enable = lib.mkEnableOption "coeusd, the coeus deployment daemon, and the `coeus` CLI";

    package = mkOption {
      type = types.package;
      default = self.packages.${pkgs.stdenv.hostPlatform.system}.default;
      defaultText = lib.literalExpression "coeus.packages.\${system}.default";
    };

    keyFile = mkOption {
      type = types.str;
      example = "/run/agenix/coeus-key";
      description = "File holding the fleet's pre-shared key (64 hex digits), read by coeusd and the CLI.";
    };

    flake = mkOption {
      type = types.str;
      example = "git+https://github.com/owner/nix";
      description = "The flake holding every machine's configuration.";
    };

    hostname = mkOption {
      type = types.nullOr types.str;
      default = config.networking.hostName;
      defaultText = lib.literalExpression "config.networking.hostName";
      description = "This machine's name in the flake; the system hostname when null.";
    };

    listen = mkOption {
      type = types.str;
      default = "[::]:45729";
    };

    sshKeyFile = mkOption {
      type = types.nullOr types.str;
      default = null;
      description = "An SSH key for private `git+ssh` flake inputs.";
    };

    history = mkOption {
      type = types.ints.unsigned;
      default = 100;
      description = "How many finished jobs to keep.";
    };

    follow = mkOption {
      default = null;
      description = "A git ref this machine catches up with on its own.";
      type = types.nullOr (
        types.submodule {
          options = {
            ref = mkOption {
              type = types.str;
              example = "deployed";
            };
            interval = mkOption {
              type = types.str;
              default = "15m";
            };
          };
        }
      );
    };

    guard = {
      enable = mkOption {
        type = types.bool;
        default = true;
        description = "Roll back an activation that leaves the machine unreachable.";
      };
      timeout = mkOption {
        type = types.str;
        default = "2m";
      };
      tailscale = mkOption {
        type = types.bool;
        default = true;
        description = "Require tailscale to be up after activating, when it is installed.";
      };
    };

    notify = mkOption {
      default = null;
      description = "Where finished jobs are announced, e.g. an ntfy topic.";
      type = types.nullOr (
        types.submodule {
          options = {
            url = mkOption { type = types.str; };
            on = mkOption {
              type = types.enum [
                "failure"
                "always"
              ];
              default = "failure";
            };
          };
        }
      );
    };

    hosts = mkOption {
      default = { };
      description = "The CLI's fleet inventory: the machines `@all` and `@<tag>` select.";
      type = types.attrsOf (
        types.submodule {
          options = {
            address = mkOption {
              type = types.nullOr types.str;
              default = null;
              description = "Hostname or IP address; the entry's name when null.";
            };
            port = mkOption {
              type = types.nullOr types.port;
              default = null;
            };
            tags = mkOption {
              type = types.listOf types.str;
              default = [ ];
            };
          };
        }
      );
    };

    settings = mkOption {
      inherit (toml) type;
      default = { };
      description = "Extra `[settings]` for `coeusd.toml`, merged over the options above.";
    };

    clientSettings = mkOption {
      inherit (toml) type;
      default = { };
      description = "Extra `[settings]` for the CLI's `coeus.toml`, merged over the options above.";
    };
  };

  config = lib.mkIf cfg.enable {
    environment.systemPackages = [ cfg.package ];

    environment.etc = {
      "coeus/coeusd.toml".source = toml.generate "coeusd.toml" daemon;
      "coeus/coeus.toml".source = toml.generate "coeus.toml" client;
    };
  };
}
