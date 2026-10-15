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
    serverConfigFile = "/run/secrets/coeus-server.toml";
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
    serverConfigFile = "/private/etc/coeus/server.toml";
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
    serverConfigFile = "/private/etc/coeus/server.toml";
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

`serverConfigFile` replaces `configFile` (a deprecated compatibility alias). It
must be absolute and supports two explicit classes:

- **Private runtime TOML:** service-user-owned, mode **0400 or 0600**; contains
  exactly one of inline `key` or an external `key_file`.
- **Public Nix-store TOML:** root-owned, regular and non-writable, without special
  mode bits; must use `key_file`. Inline PSKs in the store are rejected. This
  cannot undo prior exposure if a secret was already placed in the store.

For public store configuration, only public settings and a runtime secret **path**
enter Nix derivations/service manifests, never the key bytes:

```nix
services.coeusd.serverConfigFile = toString (pkgs.writeText "coeus-server.toml" ''
  [settings]
  key_file = "/run/secrets/coeus-psk"
  host = "127.0.0.1"
  port = 45729
  repo_url = "https://github.com/example/nix"
  state_directory = ".coeus-state"
'');
```

Provision the external PSK separately (agenix or another runtime secret manager).
`key_file` must be absolute, resolve outside `/nix/store`, and name a regular
service-user-owned file of mode **0400 or 0600**, with no special mode bits.
It contains exactly **64 ASCII hexadecimal characters**, optionally followed by
**one LF** (no CRLF, spaces, TOML or expansion). Both configuration and external
secret may use runtime-secret symlinks. Resolution is followed by no-follow,
nonblocking descriptor validation and bounded reads (64 KiB TOML, 65 bytes key).
FIFOs/devices/directories, missing inputs, wrong ownership/modes and store-backed
keys fail without default generation, config creation or secret writeback.
Parent directories and same-UID writers remain trusted; this is not race isolation
from a malicious service account. Do not print or serialize resolved settings.

Both Linux and Darwin service startup and independent bootstrap/switch use the
same immutable loader. The Linux shell entrypoint remains but delegates validation
to native `coeusd`, just like Darwin. HOME ownership and non-root service identity,
private mode-0700 working directory, umask and working-directory checks remain.
The service user and independent activation operator must be able to read the
preprovisioned secret. Rotation requires deliberately synchronizing peers and
restarting; there is no generated fallback. A changed public store config path is
a service identity change and retains the manual-switch rules.

### Client configuration for Home Manager users

When the NixOS or nix-darwin Home Manager module is imported, an optional
`services.coeusd.clientConfigFile = "/run/secrets/coeus-client.toml";` adds a
Home Manager activation entry to **every configured `home-manager.users` user**.
It works on client-only hosts too (`services.coeusd.enable = false`).
It copies the supplied runtime file to `${xdg.configHome}/coeus/config.toml`
(normally `~/.config/coeus/config.toml`). Coeus uses that XDG location by default;
`coeus --config PATH ...` still overrides it.

```toml
[settings]
key = "REPLACE_WITH_64_HEX_CHARACTERS_FROM_openssl_rand_-hex_32"
fleet = ["lab", "192.0.2.10:45729"]
default_revision = "main"
```

The exact source content is copied **at activation time**, never read by the Nix
evaluator, embedded in a derivation or passed to `home.file.*.source`. The source
must be an absolute runtime path; store paths (including resolved symlinks) are
rejected. The destination directory is mode **0700**, file **0600** and owned by
the activating Home Manager user. Replacement uses a private temporary file and
atomic rename. Existing Coeus directory/file symlinks are refused. A dry-run does
not read the source or write anything. Removing the option stops distribution;
it deliberately does not delete previously provisioned credentials.

**Provision source access deliberately:** each Home Manager activation runs as
its own user, not root. A root-only `0600` source is not readable by other users
and activation will fail, rather than weakening source permissions or writing
as root into user-controlled home paths. A runtime secret manager can supply a
restricted group-readable source for the intended users. All recipients gain
Coeus deployment authority through this shared PSK. Users outside
`home-manager.users` are not touched; without Home Manager imported this option
has no distribution effect. No implicit Home Manager dependency is added to hosts.

`clientConfigFile` remains private runtime-only configuration; server `key_file`
support does not publish or change client credentials. The distribution helper
never reads secret contents at Nix evaluation time.

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

## Darwin native entrypoint and privacy permission limits

The Darwin module starts the exact package's native `bin/coeusd` as
`ProgramArguments[0]`, not a shell trampoline. Paired `--service-home` and
`--service-state-directory` arguments enforce startup checks in-process before
configuration loading, ledger access or listening: non-root effective UID;
service-owned home and private mode-0700 working directory; readable, regular,
private service-owned mode-0400/0600 config, or immutable public store TOML
referencing a validated external runtime key as described above. It follows
runtime-secret links, sets umask 077, and enters the configured working directory.
A failed check exits without creating config or opening the ledger. Linux's
shell entrypoint delegates to the same native validator; the restricted exact-closure
elevation helper is unchanged.

This simplifies the LaunchDaemon's initial executable and macOS TCC responsible
process chain, following the [Munki 7 launchd design](https://github.com/munki/munki/wiki/Munki-7-launchd-job-changes).
It does **not** grant App Management/Full Disk Access, establish which process
macOS attributes Coeus activation to, or prove unattended application updates work.
The [Munki PPPC guidance](https://github.com/munki/munki/wiki/PPPC-Privacy-permissions)
requires permissions appropriate to the background execution identity; it is not
a substitute for an actual Coeus deployment test.

A stable native path alone is not upgrade-stable permission identity. Current
ad-hoc-signed Nix builds use a cdhash designated requirement, which changes when
the binary changes. Apple's [TN3127](https://developer.apple.com/documentation/technotes/tn3127-inside-code-signing-requirements)
explains why persistent privacy grants rely on designated requirements. A suitable
stable signing identity/designated requirement and one-time consent (or managed
PPPC policy) would need separate design and verification; this module neither
creates a signing key nor claims that ad-hoc signing confers consent. For bare
binaries, Apple's [PPPC schema](https://github.com/apple/device-management/blob/release/mdm/profiles/com.apple.TCC.configuration-profile-policy.yaml)
also identifies the installation path, so Nix-store path changes remain an
upgrade concern even with stable signing. Manual permission retention and a
second-version/path upgrade require real platform tests. Removing
App Management checks or skipping app updates is not part of this change.

## Module regression checks

`nix build .#checks.x86_64-linux.modules` evaluates genuine NixOS and nix-darwin
services without activation, including deprecated-option compatibility and hosts
without Home Manager. `nix build .#checks.x86_64-linux.client-config` evaluates
the real Home Manager integrations for both platforms and multiple users, then
runs unprivileged runtime-copy fixtures. The Home Manager check alone fetches
its fixed test-only source (revision `54ad640b81c5e9a32db1a29e7561fda628601117`,
content hash in `nix/tests/home-manager.nix`); imported service modules do not.
These checks do not install a service, prove macOS privacy consent or exercise a
real user's Home Manager activation.
