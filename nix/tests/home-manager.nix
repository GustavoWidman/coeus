{
  inputs,
  pkgs,
  system,
}:
let
  inherit (inputs.nixpkgs) lib;
  # Test-only pin; importing Coeus on a host never fetches Home Manager.
  homeManager = builtins.fetchTarball {
    url = "https://github.com/nix-community/home-manager/archive/54ad640b81c5e9a32db1a29e7561fda628601117.tar.gz";
    sha256 = "sha256-X9iN67cdW9KqLuvbyyr9rdF4Zssk1uFHh1UC/GZH08s=";
  };
  make =
    platform: withClient: enabled:
    let
      userHome = if platform == "darwin" then "/Users" else "/home";
      module =
        if platform == "darwin" then inputs.self.darwinModules.coeusd else inputs.self.nixosModules.coeusd;
      service =
        { lib, ... }:
        {
          networking.hostName = "fixture";
          system.stateVersion = if platform == "darwin" then 6 else "26.05";
          services.coeusd = {
            enable = enabled;
            clientConfigFile = if withClient then "/run/secrets/client.toml" else null;
          }
          // lib.optionalAttrs enabled { serverConfigFile = "/run/secrets/server.toml"; }
          // lib.optionalAttrs (platform == "darwin") { uid = 498; };
          users.users = lib.genAttrs [ "alice" "bob" ] (
            name:
            {
              home = "${userHome}/${name}";
            }
            // lib.optionalAttrs (platform == "linux") { isNormalUser = true; }
          );
          home-manager = {
            useGlobalPkgs = true;
            users = lib.genAttrs [ "alice" "bob" ] (
              name:
              {
                home.stateVersion = "26.05";
              }
              // lib.optionalAttrs (name == "bob") {
                xdg.configHome = "${userHome}/${name}/custom-config";
              }
            );
          };
        }
        // lib.optionalAttrs (platform == "linux") {
          boot.loader.grub.enable = false;
          fileSystems."/" = {
            device = "/dev/fixture";
            fsType = "ext4";
          };
        };
      modules = [
        module
        (homeManager + (if platform == "darwin" then "/nix-darwin" else "/nixos"))
        service
      ];
    in
    if platform == "darwin" then
      (inputs.nix-darwin.lib.darwinSystem {
        system = "aarch64-darwin";
        inherit modules;
      }).config
    else
      (inputs.nixpkgs.lib.nixosSystem {
        system = "x86_64-linux";
        inherit modules;
      }).config;
  linux = make "linux" true true;
  darwin = make "darwin" true true;
  activation = config: user: config.home-manager.users.${user}.home.activation;
  facts = {
    linuxSystem = builtins.unsafeDiscardStringContext linux.system.build.toplevel.drvPath;
    darwinSystem = builtins.unsafeDiscardStringContext darwin.system.build.toplevel.drvPath;
  };
  checks =
    lib.flatten (
      map
        (
          config:
          map
            (
              user:
              let
                entry = (activation config user).coeusClientConfig;
              in
              [
                (entry.after == [ "writeBoundary" ])
                (lib.hasInfix "source_config=${lib.escapeShellArg "/run/secrets/client.toml"}" entry.data)
                (lib.hasInfix (if user == "bob" then "/${user}/custom-config" else "/${user}/.config") entry.data)
                (!(lib.hasInfix "synthetic-only" entry.data))
                (!(config.home-manager.users.${user}.xdg.configFile ? "coeus/config.toml"))
              ]
            )
            [
              "alice"
              "bob"
            ]
        )
        [
          linux
          darwin
        ]
    )
    ++ [
      (!(activation (make "linux" false true) "alice" ? coeusClientConfig))
      (!(activation (make "darwin" false true) "alice" ? coeusClientConfig))
      (activation (make "linux" true false) "alice" ? coeusClientConfig)
      (activation (make "darwin" true false) "alice" ? coeusClientConfig)
      (!((make "linux" true false).systemd.services ? coeusd))
      (!((make "darwin" true false).launchd.daemons ? coeusd))
    ];
in
assert lib.assertMsg (builtins.all (
  x: x
) checks) "Coeus Home Manager integration regression failed";
pkgs.runCommand "coeus-client-config-${system}"
  {
    nativeBuildInputs = [
      pkgs.python3
      pkgs.bash
      pkgs.coreutils
    ];
  }
  ''
    python3 ${./client_config.py} ${../modules/client-config.sh} ${pkgs.bash}/bin/bash ${pkgs.coreutils}/bin ${../modules/client-config.sh}
    cat > $out <<'JSON'
    ${builtins.toJSON facts}
    JSON
  ''
