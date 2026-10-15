# Nix services: Linux and macOS

The flake exports `packages.<system>.coeus` (also `default`),
`nixosModules.coeusd` / `nixosModules.default`, and
`darwinModules.coeusd` / `darwinModules.default`. Linux means **NixOS** here,
not a generic non-NixOS systemd installer. macOS uses a **launchd daemon**,
not systemd or a login-only agent. Both modules default to disabled. Packages/checks support x86_64-linux,
aarch64-linux and aarch64-darwin. The pinned Nixpkgs 26.11 has dropped Intel
Darwin support; this flake does not advertise an unusable x86_64-darwin output.

## Add it to a host flake

Pin a reviewed Coeus revision in the host's lock file. These examples describe
configuration; they do not install or deploy anything by themselves.

```nix
{
  inputs.coeus.url = "github:GustavoWidman/coeus/feat/async-deployment-jobs";
  inputs.coeus.inputs.nixpkgs.follows = "nixpkgs";
  # In the appropriate nixosSystem/darwinSystem modules list:
  # inputs.coeus.nixosModules.coeusd or inputs.coeus.darwinModules.coeusd
}
```

NixOS dedicated identity:

```nix
{
  services.coeusd = {
    enable = true;
    configFile = "/run/secrets/coeus-server.toml";
    enablePasswordlessActivation = true;
  };
}
```

Darwin dedicated identity:

```nix
{
  services.coeusd = {
    enable = true;
    uid = 498; # EXAMPLE ONLY: allocate an unused UID on your host first
    configFile = "/private/etc/coeus/server.toml";
    enablePasswordlessActivation = true;
  };
}
```

The dedicated account defaults to `coeus`, HOME `/var/lib/coeus`, private
working directory `/var/lib/coeus/state`; Darwin uses the existing `staff`
group (GID 20). A custom Darwin primary group must already exist; set both
`group` and `gid` consistently. Account and state-directory creation are
part of a separately authorized host activation, not this source change.
Darwin prepares the private working/log directory after user setup and before
launchd loads the daemon, including on a first installation.

To reuse an existing user's Git/SSH configuration **without running as root**:

```nix
{
  services.coeusd = {
    enable = true;
    createUser = false;
    user = "deploy";
    group = "staff"; # e.g. "users" on NixOS; actual existing group required
    home = "/Users/deploy"; # e.g. /home/deploy on NixOS
    stateDirectory = "/var/lib/coeus/state";
    configFile = "/private/etc/coeus/server.toml";
    enablePasswordlessActivation = true;
  };
}
```

HOME must actually belong to that identity. Builds inherit **that user's**
Git configuration, SSH includes, keys and agent access. A dedicated identity
needs its own credentials provisioned explicitly; pointing HOME at someone
else's home or borrowing root's configuration is not supported. A service
cannot assume an interactive shell's `SSH_AUTH_SOCK` is stable or accessible
at boot. `environment.SSH_AUTH_SOCK` can name a deliberately managed socket;
do not store keys, tokens or other secrets in `environment`. `extraPackages`
adds runtime tools. The generated colon-separated PATH includes nh, Nix,
Git, SSH and common shell tools; Darwin also includes system tool paths.
The module owns HOME/USER/LOGNAME/PATH and the nh elevation strategy.

## Private runtime configuration and durable state

`configFile` is required when enabled, must be absolute, and must **not**
be a Nix store path. Do not use `pkgs.writeText` for a real PSK. Provision it
through a runtime secret manager or a manual private installation. The
resolved file must be owned by the service user with mode **0600 or 0400**;
group/world-readable files, a symlink into `/nix/store`, missing files and
root identities fail startup. HOME ownership and working-directory owner /
mode 0700 are checked too. Startup does not fall back to embedded defaults.

Runtime TOML (replace the placeholder with a newly generated 32-byte PSK,
shared only with authorized clients; never use the tracked public dev key):

```toml
[settings]
key = "REPLACE_WITH_64_HEX_CHARACTERS_FROM_openssl_rand_-hex_32"
host = "127.0.0.1"
port = 45729
repo_url = "https://github.com/example/host-configuration"
state_directory = ".coeus-state"
```

Use a relative `state_directory` to keep the ledger under the module's private
working directory. An absolute TOML path is operator-controlled and must have
its own correct permissions. Changing the module working directory or TOML
ledger path **does not migrate history**; stop an idle daemon, preserve and
migrate its entire ledger deliberately. Never delete it merely to unblock
admission or silently replay interrupted jobs. See [durable-job limits](async-deployments.md).
The daemon has a 128-record durable cap. General Interrupted jobs still block new
admission pending operator reconciliation; independent activation receipts have
a verification-only `coeus reconcile JOB` path.

The module does not open a firewall, enable a Tailscale route, or grant Nix
`trusted-users`. Explicitly configure transport exposure and any required
Nix-daemon access on the host. A PSK authorizes deployment/cancel operations:
treat it as a root control-plane credential, not an ordinary app password.

## Passwordless nh, without root builds

`enablePasswordlessActivation` defaults to **false**. Opting in installs a
NOPASSWD sudoers rule for an immutable, command-specific activation helper.
This is the passwordless elevation used by nh; it is deliberately **not**
`NOPASSWD: ALL`, `NOPASSWD: env`, or a rule for `sudo nh switch`.

A rule allowing just `nh` is ineffective: nh internally elevates profile and
activation commands, not another nh invocation. Running all of nh as root
also evaluates/builds with root credentials and repeats the HOME / Git SSH
ownership failures. With this module, checkout/evaluation/build stay
non-root. The custom `NH_ELEVATION_STRATEGY` adapter strips nh's literal
`env KEY=VALUE...` envelope without evaluating or forwarding its assignments,
then executes absolute `sudo -n -- STORE_HELPER COMMAND ARGS`. No password
prompt is possible. Only the root helper is authorized by sudoers.

The root helper validates exact argv and resolves a realised closure once:

- `nix build --no-link --profile /nix/var/nix/profiles/system CLOSURE`;
  no flake URL, other profile, extra Nix options or root flake evaluation.
- NixOS `CLOSURE/bin/switch-to-configuration test` or `boot`.
  nh's switch sequence is **test → profile update → boot**.
- Darwin `CLOSURE/sw/bin/darwin-rebuild activate`. The closure-relative `$0`
  spelling is preserved even when the script itself is a store symlink.

The closure must resolve to one canonical `/nix/store/HASH-NAME` system root
with an executable store-backed activation marker; traversal, dangling paths,
non-store paths and arbitrary commands are rejected. Root execution gets a
fresh environment: root HOME/USER/LOGNAME, module-owned PATH, NIX_REMOTE=daemon.
User HOME/PATH, NIX_CONFIG, GIT_* and NH_* are not forwarded. sudo's own
SUDO_USER is retained for legacy Darwin user activation, not accepted from
the stripped nh envelope. The immutable helper command path is what the
sudoers rule grants; nh's wrapped vs unwrapped executable is not authorized
as root and therefore does not change this rule's correctness.

**This is still root-equivalent deployment authority.** The service account
can build a system closure containing arbitrary root activation code. The
helper prevents accidental root builds / arbitrary command shapes; it is
not a sandbox against a malicious authorized repository, account or client.

The tested inputs provide nh **4.4.2** and nix-darwin
`4cff07de74b50e64bdd68cd4e722ab5b6b35ee48` (a test-only input following the
flake's existing nixpkgs). Imported host modules use the host's nixpkgs;
`nhPackage` and `nixPackage` are configurable. Different nh argv or external
Darwin versions need separate compatibility verification. Older Darwin
systems with a nondeprecated `activate-user` take nh's unprivileged activation
branch and are **not supported by this elevation integration**. The separate
Determinate Nix / nom activity-type incompatibility is not fixed here.

## Bootstrap and daemon-changing deployments

See [independent activation](independent-activation.md) for the implemented exact-closure
manual-switch workflow, supported SSH bootstrap path, migration prerequisites,
receipt reconciliation, and native-platform validation limits. Never run a legacy
`nh switch` through the daemon being replaced.
