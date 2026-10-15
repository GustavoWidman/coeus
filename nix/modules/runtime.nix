{
  cfg,
  pkgs,
  lib,
  platform,
  configHost,
  accountUid ? null,
  accountGid ? null,
}:
let
  packages = [
    cfg.nhPackage
    cfg.nixPackage
    pkgs.git
    pkgs.openssh
    pkgs.coreutils
    pkgs.bash
    pkgs.gnugrep
    pkgs.gnused
    pkgs.findutils
  ]
  ++ cfg.extraPackages;
  path =
    lib.makeBinPath packages
    + lib.optionalString (platform == "darwin") ":/usr/bin:/bin:/usr/sbin:/sbin";
  sudo = if platform == "darwin" then "/usr/bin/sudo" else "/run/wrappers/bin/sudo";
  args =
    mode: helper:
    lib.escapeShellArgs [
      mode
      platform
      "${cfg.nixPackage}/bin/nix"
      sudo
      helper
      path
    ];
  helper = pkgs.writeShellScript "coeus-nh-activate" ''
    exec ${pkgs.python3}/bin/python3 -I ${../elevation.py} ${args "helper" "unused"} "$@"
  '';
  adapter = pkgs.writeShellScript "coeus-nh-elevate" ''
    exec ${pkgs.python3}/bin/python3 -I ${../elevation.py} ${args "adapter" (toString helper)} "$@"
  '';
  manifestData = {
    # An unset nix-darwin hostName is null. Never generate an omitted TOML host
    # or guess runtime identity: the enabled-module assertion below explains it.
    host = if configHost == null then "" else configHost;
    inherit platform;
    daemon = "${cfg.package}/bin/coeusd";
    version = lib.getVersion cfg.package;
    protocol = 6;
    user = cfg.user;
    account = {
      managed = cfg.createUser;
    }
    // lib.optionalAttrs (cfg.createUser && accountUid != null) { uid = accountUid; }
    // lib.optionalAttrs (cfg.createUser && accountGid != null) { gid = accountGid; };
    state_directory = cfg.stateDirectory;
    config_file = cfg.serverConfigFile;
    runner = toString runner;
    elevation_adapter = toString adapter;
    nix = "${cfg.nixPackage}/bin/nix";
  };
  manifest = (pkgs.formats.toml { }).generate "coeus-service.toml" manifestData;
  environment = cfg.environment // {
    COEUS_SERVICE_MANIFEST = toString manifest;
    HOME = cfg.home;
    USER = cfg.user;
    LOGNAME = cfg.user;
    PATH = path;
    NH_ELEVATION_STRATEGY = if cfg.enablePasswordlessActivation then toString adapter else "none";
  };
  # A native initial executable gives launchd/TCC a concrete daemon identity.
  # Keep Linux's shell entrypoint. Both platforms perform shared checks in Rust.
  runner =
    if platform == "darwin" then
      "${cfg.package}/bin/coeusd"
    else
      pkgs.writeShellScript "coeusd-start" ''
        set -eu
        # Keep the Linux shell entrypoint/lifetime, but use the same native
        # validator as Darwin and independent activation (including key_file).
        exec ${cfg.package}/bin/coeusd \
          --service-home ${lib.escapeShellArg cfg.home} \
          --service-state-directory ${lib.escapeShellArg cfg.stateDirectory} \
          --config ${lib.escapeShellArg cfg.serverConfigFile}
      '';
  assertions = [
    {
      assertion =
        builtins.isString configHost
        && builtins.match "[a-zA-Z0-9_-]+" configHost != null
        && builtins.stringLength configHost < 128;
      message = "Coeus requires an explicit networking.hostName (1-127 letters, digits, hyphen or underscore) for verified target identity";
    }
    {
      assertion =
        builtins.match "[a-z_][a-z0-9_-]*" cfg.user != null
        && builtins.match "[a-z_][a-z0-9_-]*" cfg.group != null;
      message = "Coeus user/group must be simple account names";
    }
    {
      assertion =
        cfg.stateDirectory != "/"
        && !(lib.hasPrefix "/nix/store" cfg.stateDirectory)
        && builtins.match "[^[:space:]]+" cfg.stateDirectory != null;
      message = "Coeus stateDirectory must be a non-store private directory without whitespace";
    }
    {
      assertion = cfg.user != "root" && cfg.user != "";
      message = "services.coeusd.user must be a non-root identity";
    }
    {
      assertion = lib.hasPrefix "/" cfg.serverConfigFile;
      message = "services.coeusd.serverConfigFile must be absolute (public store TOML requires an external runtime key_file)";
    }
    {
      assertion =
        lib.hasPrefix "/" cfg.home
        && cfg.home != "/"
        && !(lib.hasPrefix "/nix/store" cfg.home)
        && lib.hasPrefix "/" cfg.stateDirectory;
      message = "Coeus HOME and stateDirectory must be absolute";
    }
    {
      assertion =
        !(builtins.any (k: builtins.hasAttr k cfg.environment) [
          "HOME"
          "PATH"
          "USER"
          "LOGNAME"
          "NH_ELEVATION_STRATEGY"
          "NH_BYPASS_ROOT_CHECK"
        ]);
      message = "Coeus identity/PATH/elevation environment is module-owned";
    }
  ];
in
{
  inherit
    packages
    path
    helper
    adapter
    environment
    runner
    assertions
    manifest
    manifestData
    ;
}
