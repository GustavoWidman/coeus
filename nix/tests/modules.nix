{
  inputs,
  pkgs,
  system,
}:
let
  inherit (inputs.nixpkgs) lib;
  linux =
    args:
    inputs.nixpkgs.lib.nixosSystem {
      system = "x86_64-linux";
      modules = [
        inputs.self.nixosModules.coeusd
        {
          system.stateVersion = "26.05";
          boot.loader.grub.enable = false;
          fileSystems."/" = {
            device = "/dev/fixture";
            fsType = "ext4";
          };
          services.coeusd = {
            enable = true;
            configFile = "/run/secrets/coeus.toml";
            enablePasswordlessActivation = true;
          }
          // args;
        }
      ];
    };
  darwin =
    args:
    inputs.nix-darwin.lib.darwinSystem {
      system = "aarch64-darwin";
      modules = [
        inputs.self.darwinModules.coeusd
        {
          system.stateVersion = 6;
          nixpkgs.config.allowUnfree = false;
          services.coeusd = {
            enable = true;
            uid = 498;
            configFile = "/private/etc/coeus/server.toml";
            enablePasswordlessActivation = true;
          }
          // args;
        }
      ];
    };
  l = (linux { }).config;
  d = (darwin { }).config;
  existingLinux =
    (linux {
      createUser = false;
      user = "deploy";
      group = "users";
      home = "/home/deploy";
    }).config;
  existingDarwin =
    (darwin {
      createUser = false;
      user = "deploy";
      home = "/Users/deploy";
    }).config;
  disabledLinux =
    (inputs.nixpkgs.lib.nixosSystem {
      system = "x86_64-linux";
      modules = [
        inputs.self.nixosModules.default
        { system.stateVersion = "26.05"; }
      ];
    }).config;
  disabledDarwin =
    (inputs.nix-darwin.lib.darwinSystem {
      system = "aarch64-darwin";
      modules = [
        inputs.self.darwinModules.default
        { system.stateVersion = 6; }
      ];
    }).config;
  invalid = args: builtins.any (a: !a.assertion) (linux args).config.assertions;
  linuxRule = builtins.head (
    builtins.filter (rule: builtins.elem "coeus" (rule.users or [ ])) l.security.sudo.extraRules
  );
  linuxCommand = builtins.head linuxRule.commands;
  # Check the fully composed root activation script, not just our fragment.
  # Both markers must exist: a missing fragment must not accidentally sort first.
  precedes =
    first: second: text:
    let
      beforeFirst = lib.splitString first text;
      beforeSecond = lib.splitString second text;
    in
    builtins.length beforeFirst == 2
    && builtins.length beforeSecond == 2
    &&
      builtins.stringLength (builtins.head beforeFirst)
      < builtins.stringLength (builtins.head beforeSecond);
  darwinDirectory =
    cfg:
    "-d -m 0700 -o ${lib.escapeShellArg cfg.services.coeusd.user} -g ${lib.escapeShellArg cfg.services.coeusd.group} ${lib.escapeShellArg cfg.services.coeusd.stateDirectory}";
  darwinLoad = "launchctl load -w '/Library/LaunchDaemons/org.nixos.coeusd.plist'";
  darwinOrdering = {
    userBeforeDirectory =
      precedes "sysadminctl -addUser coeus " (darwinDirectory d)
        d.system.activationScripts.script.text;
    directoryBeforeLaunchd =
      precedes (darwinDirectory d) darwinLoad
        d.system.activationScripts.script.text;
    existingUserDirectoryBeforeLaunchd =
      precedes (darwinDirectory existingDarwin) darwinLoad
        existingDarwin.system.activationScripts.script.text;
    disabledHasNoDirectory =
      !(lib.hasInfix (darwinDirectory d) disabledDarwin.system.activationScripts.script.text);
  };
  facts = {
    # Force genuine full NixOS and nix-darwin evaluation, not stubbed option sets.
    linuxSystem = builtins.unsafeDiscardStringContext l.system.build.toplevel.drvPath;
    darwinSystem = builtins.unsafeDiscardStringContext d.system.build.toplevel.drvPath;
    linuxService = l.systemd.services.coeusd.serviceConfig;
    darwinService = d.launchd.daemons.coeusd.serviceConfig;
    linuxSudo = linuxRule;
    darwinSudo = d.security.sudo.extraConfig;
    inherit darwinOrdering;
  };
  checks = (builtins.attrValues darwinOrdering) ++ [
    (linuxRule.users == [ "coeus" ] && linuxCommand.options == [ "NOPASSWD" ])
    (lib.hasSuffix "-coeus-nh-activate" linuxCommand.command)
    (builtins.length linuxRule.commands == 1)
    (
      !(builtins.any (
        rule: builtins.elem "coeus" (rule.users or [ ])
      ) disabledLinux.security.sudo.extraRules)
    )
    (l.systemd.services.coeusd.serviceConfig.User == "coeus")
    (
      !l.systemd.services.coeusd.restartIfChanged
      && !l.systemd.services.coeusd.stopIfChanged
      && !l.systemd.services.coeusd.reloadIfChanged
    )
    (l.systemd.services.coeusd.environment.HOME == "/var/lib/coeus")
    (!(l.systemd.services.coeusd.serviceConfig.NoNewPrivileges or false))
    (d.launchd.daemons.coeusd.serviceConfig.UserName == "coeus")
    (d.users.users.coeus.uid == 498)
    (existingLinux.systemd.services.coeusd.environment.HOME == "/home/deploy")
    (!(builtins.hasAttr "deploy" existingLinux.users.users))
    (existingDarwin.launchd.daemons.coeusd.serviceConfig.EnvironmentVariables.HOME == "/Users/deploy")
    (!(builtins.hasAttr "deploy" existingDarwin.users.users))
    (!(builtins.hasAttr "coeusd" disabledLinux.systemd.services))
    (!(builtins.hasAttr "coeusd" disabledDarwin.launchd.daemons))
    (!disabledLinux.services.coeusd.enable && !disabledDarwin.services.coeusd.enable)
    (invalid { user = "root"; })
    (invalid { configFile = "/nix/store/secret.toml"; })
    (invalid { configFile = "relative.toml"; })
    (invalid { environment.PATH = "/usr/bin"; })
    (builtins.any (a: !a.assertion) (darwin { uid = null; }).config.assertions)
  ];
in
assert lib.assertMsg (builtins.all (x: x) checks) "Coeus module regression failed";
pkgs.runCommand "coeus-module-evaluation-${system}"
  { nativeBuildInputs = lib.optionals (!pkgs.stdenv.isDarwin) [ pkgs.sudo ]; }
  ''
    cat > darwin.sudoers <<'RULE'
    ${builtins.unsafeDiscardStringContext d.security.sudo.extraConfig}
    RULE
    cat > linux.sudoers <<'RULE'
    coeus ALL=(root) NOPASSWD: ${builtins.unsafeDiscardStringContext linuxCommand.command}
    RULE
    ${if pkgs.stdenv.isDarwin then "/usr/sbin/visudo" else "visudo"} -c -f darwin.sudoers
    ${if pkgs.stdenv.isDarwin then "/usr/sbin/visudo" else "visudo"} -c -f linux.sudoers
    cat > $out <<'JSON'
    ${builtins.unsafeDiscardStringContext (builtins.toJSON facts)}
    JSON
  ''
