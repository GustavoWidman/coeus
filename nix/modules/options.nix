self:
{
  config,
  lib,
  pkgs,
  ...
}:
let
  inherit (lib) mkOption types;
  cfg = config.services.coeusd;
in
{
  options.services.coeusd = {
    enable = lib.mkEnableOption "Coeus deployment daemon";
    package = mkOption {
      type = types.package;
      default = self.packages.${pkgs.stdenv.hostPlatform.system}.coeus;
      description = "Coeus client and daemon package.";
    };
    configFile = mkOption {
      type = types.str;
      description = "Absolute runtime TOML path outside /nix/store, owned by the service user with mode 0600 or 0400. Never put a PSK in a Nix expression.";
    };
    user = mkOption {
      type = types.str;
      default = "coeus";
      description = "Non-root service identity; choose an existing user to reuse that user's Git/SSH configuration.";
    };
    group = mkOption {
      type = types.str;
      default = if pkgs.stdenv.isDarwin then "staff" else cfg.user;
      description = "Service primary group.";
    };
    createUser = mkOption {
      type = types.bool;
      default = true;
      description = "Create a dedicated service user; disable when using an existing account.";
    };
    home = mkOption {
      type = types.str;
      default = "/var/lib/coeus";
      description = "Actual home owned by the service identity, not root's or another user's home.";
    };
    stateDirectory = mkOption {
      type = types.str;
      default = "/var/lib/coeus/state";
      description = "Private daemon working directory. Use a relative state_directory in the runtime TOML to place the durable ledger here.";
    };
    nhPackage = mkOption {
      type = types.package;
      default = pkgs.nh;
      description = "nh package. This integration is validated against nh 4.4.2.";
    };
    nixPackage = mkOption {
      type = types.package;
      default = pkgs.nix;
      description = "Nix used by both unprivileged builds and the profile-update helper.";
    };
    extraPackages = mkOption {
      type = types.listOf types.package;
      default = [ ];
      description = "Additional runtime tools.";
    };
    environment = mkOption {
      type = types.attrsOf types.str;
      default = { };
      description = "Extra environment (e.g. a service-owned SSH_AUTH_SOCK). Do not put credentials here: values enter the store.";
    };
    enablePasswordlessActivation = mkOption {
      type = types.bool;
      default = false;
      description = "Explicitly grant this identity root-equivalent deployment authority through the restricted nh activation helper.";
    };
  };
}
