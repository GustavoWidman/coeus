# Fleet client (protocol v6)

Client and daemon must be upgraded together. The Noise prologue rejects old peers.
This iteration is local source work, not permission provisioning or a real-host
activation test. On Darwin the responsible native `coeusd` still needs macOS approval
for the app writes performed during activation. This does not preserve FDA consent
across changing ad-hoc signatures/Nix-store paths.

## Configuration and migration

The client reads `$XDG_CONFIG_HOME/coeus/config.toml`, or
`~/.config/coeus/config.toml` when XDG_CONFIG_HOME is unset. `--config FILE` overrides
that choice. Missing/invalid files fail; the client never generates credentials or
rewrites configuration. Do not copy actual keys into documentation or the Nix store.

```toml
[settings]
key = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
default_revision = "main"
fleet = ["lab", "192.0.2.10:45729"]
```

The key above is illustrative, not a production credential. A shared key authorizes
deployment, cancellation and logs for all fleet endpoints; provision it privately.
An omitted default_revision means `main`. An absent/empty fleet is valid for explicit
target commands, but bare `deploy`/`status` fail rather than silently using localhost.
The old client `host`/`port` settings no longer select a machine: migrate them into
`fleet = ["HOST:PORT"]` or pass the target explicitly. Old fields are ignored, not
interpreted as implicit fleet membership. Server settings are separate: public server
TOML can now reference an external private runtime `key_file`; this does not make the
client key safe for the Nix store. Existing private inline server keys remain supported.
See [Nix services](nix-services.md) for the server format and runtime-secret-safe client
file distribution.

## Targets and authentication

`coeus deploy MACHINE` accepts a hostname, IPv4, raw IPv6 (default port), or
`[IPv6]:PORT`. Hostname aliases are resolved with local OpenSSH `ssh -G`, including
the user's usual SSH config, then DNS. This evaluates local SSH config (including
any user-configured Match exec); it does not open an SSH connection. OpenSSH
`HostName` is used; SSH `Port`, `User` and ProxyCommand are **not** Noise endpoint
configuration. Coeus defaults to port 45729; `MACHINE:PORT` overrides it.
`local`, `localhost` and `me` always map to IPv4 127.0.0.1, without SSH/DNS overrides.
Unknown or malformed targets fail. Fleet is capped at 128 targets and uses at most
8 concurrent operations. Duplicate names or resolved socket endpoints are rejected
rather than deploying the same endpoint twice (different DNS addresses are not a
cryptographic machine identity).

Every connection requires the existing PSK Noise handshake and authenticated hello
with matching protocol before deployment/status/output. An open but wrong-key,
wrong-protocol or malformed endpoint never triggers bootstrap. TCP connection
failure is distinguished from authentication failure. Requests are bounded by a
15-second deadline; connection/framing cancellation poisons that connection.
A lost deployment ACK is ambiguous: **query status; never blindly submit again**.
The client flushes the target and random retry nonce before sending. The daemon
assigns ascending job IDs; nonce and job ID are distinct. To retry a reviewed lost
ACK use the **same target, nonce and options** with `--request-id NONCE`; explicit
nonces are disallowed for a whole-fleet request.

## Command behavior

```sh
coeus deploy me                      # configured revision, one daemon
coeus deploy lab --rev COMMIT         # interactive: follow accepted job to terminal
coeus deploy                         # configured fleet; interactive real-time view
coeus deploy --no-tty                 # dispatch fleet, ACKs/errors, then exit
coeus status                         # fleet table, never a TUI
coeus status lab                      # active job or last-active job with details
coeus status lab --job 2 --watch       # explicit historical job inspection
coeus attach lab                      # shared raw output across future jobs
coeus attach lab --live                # skip retained history
coeus logs 2 --target lab --follow     # durable compact history for one job
coeus cancel 2 --target lab            # explicit cancellation request, not rollback
```

`logs`/`cancel` may omit `--target` only if fleet has exactly one member. A status
row's activity `Completed` means *idle with retained activity*, not successful
deployment: its separate outcome remains Failed/Cancelled/Interrupted/Succeeded.
It does not dump all historical jobs. Queued work and manual-switch barriers retain
their actual phase. Structured status output remains visible at quiet verbosity.

Only interactive stdin **and** stdout enable auto-follow. `--no-tty` or noninteractive
execution dispatches and exits after ACKs/errors; it never prompts or opens a TUI.
Single-machine deploy follows only its accepted job (including initial replay),
auto-detaching at its terminal state. Fleet deploy dispatches boundedly and opens
an alternate-screen table refreshed in real time. Ctrl+C detaches without cancelling;
accepted jobs belong to their daemons. The table restores alternate screen/cursor
on normal/error/detach exits; SIGKILL/power loss cannot run cleanup. Failed dispatches
are individually reported without retrying accepted or ambiguous requests.

Explicit `attach TARGET` has no job ID and stays attached across job boundaries.
All subscribers consume the same bounded daemon-global volatile byte feed with job
metadata, not a per-job status polling substitute. Raw bytes retain ANSI/CR/UTF-8
and are not sanitized; use only with trusted authenticated producers. The global
cursor is reset on daemon restart. History eviction errors require an explicit
`--live` reattach; slow/disconnected consumers cannot block deployments. Per-job raw
replay remains the internal API for deploy's accepted-job follow; durable logs use
a separate cursor. Ctrl+C/output errors detach only; stdin and signals are never
forwarded. Local output settings/flags are restored on exit.

## Optional interactive bootstrap

Unreachable single-target interactive deployment can offer the existing SSH bootstrap
engine only with an explicit per-target prerequisite entry. Fleet/non-TTY/no-tty
commands never prompt. For example (paths/identity must already exist on target):

```toml
[bootstrap.lab]
activation_directory = "/var/lib/coeus/state/.coeus-state.activation"
peer_config = "/private/etc/coeus/server.toml"
flake = "github:OWNER/CONFIG/FULL_COMMIT"
hostname = "lab"
nix_binary = "/nix/var/nix/profiles/default/bin/nix"
operator_sudo = true
# copy_binary = "/path/to/explicit/matching-target/coeus"
```

The entry key is the exact target string (quote it for a target containing a port).
The SSH host remains the alias without the Coeus port; reserved local aliases use
127.0.0.1. The default answer is no. Existing strict host-key checking, selected
matching binary, absolute paths, explicit Nix, existing credentials/account and
scoped/existing sudo policy remain mandatory. No key installation, permission grants,
secret copying, assumed fresh host or automatic deployment retry is added. A successful
bootstrap must pass a new authenticated hello before the requested deployment is sent.
See [independent activation](independent-activation.md) for unsupported bootstrap cases.
