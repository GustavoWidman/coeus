{
  cfg,
  pkgs,
  lib,
  platform,
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
  environment = cfg.environment // {
    HOME = cfg.home;
    USER = cfg.user;
    LOGNAME = cfg.user;
    PATH = path;
    NH_ELEVATION_STRATEGY = if cfg.enablePasswordlessActivation then toString adapter else "none";
  };
  runner = pkgs.writeShellScript "coeusd-start" ''
    set -eu
    uid=$(${pkgs.coreutils}/bin/id -u)
    test "$uid" != 0 || { echo "coeusd must not run as root" >&2; exit 1; }
    test "$(${pkgs.coreutils}/bin/stat -Lc %u -- ${lib.escapeShellArg cfg.home})" = "$uid"
    test "$(${pkgs.coreutils}/bin/stat -Lc %u -- ${lib.escapeShellArg cfg.stateDirectory})" = "$uid"
    test "$(${pkgs.coreutils}/bin/stat -Lc %a -- ${lib.escapeShellArg cfg.stateDirectory})" = 700
    config=${lib.escapeShellArg cfg.configFile}
    actual=$(${pkgs.coreutils}/bin/realpath -e -- "$config")
    case "$actual" in /nix/store/*) echo "coeusd config must not be in the store" >&2; exit 1;; esac
    test -f "$actual" && test -r "$actual"
    test "$(${pkgs.coreutils}/bin/stat -Lc %u -- "$actual")" = "$(${pkgs.coreutils}/bin/id -u)"
    mode=$(${pkgs.coreutils}/bin/stat -Lc %a -- "$actual")
    test "$mode" = 600 || test "$mode" = 400
    umask 077
    cd ${lib.escapeShellArg cfg.stateDirectory}
    exec ${cfg.package}/bin/coeusd --config "$actual"
  '';
  assertions = [
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
      assertion = lib.hasPrefix "/" cfg.configFile && !(lib.hasPrefix "/nix/store/" cfg.configFile);
      message = "services.coeusd.configFile must be an absolute runtime path outside /nix/store";
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
    ;
}
