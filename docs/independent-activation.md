# bootstrap and daemon updates

Coeus builds a **single exact system closure** with `nix build --out-link`.
Activation never evaluates the flake again. The private out-link is an indirect
Nix GC root, retained even on cancellation/failure; do not remove it until the
receipt has been reconciled and the system is independently protected by a profile.
Archival/GC-root cleanup is deliberately manual, not an automatic timeout.

The module adds `coeus-service.toml` to the system closure. Detection compares the
running and built closure's daemon executable, immutable runner (config path,
HOME, user, tools and elevation policy), and **actual generated systemd unit or
launchd plist**, including overrides. It is not a source-revision heuristic.
Missing manifests, disabled/removed Coeus service, incompatible protocols, account
or state-directory migration (including declared managed UID/GID/account-lifecycle
policy changes) are rejected **before activation**. Runtime secret
contents are neither copied into the store nor changed by this mechanism. Changes
to those files outside a system build remain an operator-managed migration.

## flake configuration trust

Normal deployment and daemon-less bootstrap builds pass `--accept-flake-config`
to Nix so a noninteractive build accepts the selected flake's `nixConfig`
settings (including substituters and public keys) without prompting. Deploy only
repositories/revisions whose flake settings you trust. This does not grant the
service user extra Nix-daemon privileges: settings restricted by the daemon's
trust policy can still be rejected or ignored. `--clean-substituters` continues
to supply explicit command-line cache/key overrides for normal deployments.

Exact-closure activation does not evaluate a flake and does **not** pass this flag.
The restricted privileged helper's argv allowlist is unchanged; this flag does
not authorize new privileged activation commands or change the manual-switch gate.

## normal deployments

`coeus deploy TARGET --rev COMMIT` keeps its durable retry nonce (distinct from the
server-assigned ascending job ID), FIFO admission, PTY
attachment, compact logs and explicit cancellation. Ordinary builds whose service
identity is unchanged activate automatically. NixOS follows pinned nh 4.4.2's
**test → exact profile update → boot** contract. Darwin updates the exact profile
then invokes `CLOSURE/sw/bin/darwin-rebuild activate`; neither uses `nh switch .`.

**`--dry-run` now means build-only**: no profile update, activation or service
restart. Its retained receipt cannot subsequently activate. Submit a new deployment nonce
without `--dry-run` to request activation. This deliberately corrects the previous
misleading flag that only disabled eval-cache and still switched the host.

## when coeusd itself changes

The daemon persists `RequiresManualSwitch`, retains the built root, and stops FIFO
before activation. It never kills itself by activating its own replacement. Status
and logs report the handoff. Requests already admitted remain queued; they cannot
pass this barrier while activation is in progress or uncertain.

From an **independent operator session**, using SSH as the same non-root identity
that owns the daemon state and credentials:

```sh
coeus switch JOB --ssh laptop \
  --activation-directory /var/lib/coeus/state/.coeus-state.activation \
  --peer-config /private/etc/coeus/server.toml
```

The directory is derived from the actual runtime TOML ledger path by adding
`.activation` to its final component (not inside the ledger). Status logs print it.
For a service run as `deploy`, select the `laptop` SSH alias configured for that
account; a separate hidden service account is not implicitly impersonated.

The local CLI uses normal OpenSSH aliases/agent/known_hosts, **strict host-key
checking**, bounded commands and POSIX script stdin; it does not disable key
checking, source shell profiles, rewrite credentials or open a new listener.
It copies this exact client executable, checks target OS/architecture and its
version/protocol via `coeus identity`, then runs the independent target-side
activation path. `--copy-binary /path/to/target/coeus` explicitly selects a matching
cross-target executable. No `latest`, paid/download fallback, channel mutation or
unfinished release/CD machinery is used. Exact-tag asset fetching is not yet
supported: select a matching executable explicitly. A copied Nix-linked binary
also needs its runtime closure available on the target; portable release builds
are a separate task.

The target checks the rooted closure and active service still match the receipt,
and authenticates that this daemon has durably handed off this job. It writes and
fsyncs `Activating` before the first privileged command. It activates the **rooted
closure**, explicitly restarts `coeusd.service` on Linux (module automatic restart
is disabled), and observes the new system closure plus a Noise-authenticated
replacement's hostname, executable, runner, version, protocol, OS/architecture and
new PID before writing `Verified`. Darwin's activation reloads the changed plist;
a new PID is required there as well. The daemon only observes this receipt.

Use the module's restricted helper by default. `--operator-sudo` explicitly uses
an operator's **existing** `sudo -n` authority for the same concrete activation
commands; it does not install a new rule. It is needed before the module helper
exists, or when upgrading an older helper that lacks scoped service restart.
No broad `sudo nh`, `sudo env` or `NOPASSWD: ALL` rule is introduced.

## bootstrap, before a daemon exists

```sh
coeus bootstrap 123456 --ssh laptop \
  --flake github:OWNER/HOST-CONFIG/FULL_COMMIT \
  --hostname laptop \
  --nix-binary /nix/var/nix/profiles/default/bin/nix \
  --activation-directory /Users/deploy/coeus-state/.coeus-state.activation \
  --peer-config /Users/deploy/coeus/server.toml \
  --operator-sudo \
  --copy-binary /path/to/matching-target/coeus
```

Bootstrap builds the target's `nixosConfigurations.HOST.config.system.build.toplevel`
or `darwinConfigurations.HOST.system` on the SSH target and feeds the **same**
receipt, privilege, activation and authenticated health-check engine. It does not
need an installed coeusd, nh or deploy.nu. It requires an already installed,
flake-capable Nix selected by the explicit absolute target `--nix-binary`
(existence/executable preflight; no SSH PATH or shell-profile assumption), native
service manager, a full GitHub commit (no floating
branch/tag), and permission to build/activate that host configuration.

Provision the existing private `[settings]` server TOML yourself (same shared key,
mode 0600/0400), and point the built module at that exact file. Bootstrap neither
generates credentials nor edits the configuration repo. The service identity and
private parent directories must already exist and match the SSH user; use
`createUser = false` for this supported first-install path. Bootstrapping a new
hidden account, migration from an already-installed legacy daemon, a non-Nix host,
installer/reboot workflows and account/secret transport are **explicitly not
supported**. Existing service definitions/listeners are rejected instead of being
replaced behind an active queue. Build the new module once from an idle independent
operator session to migrate a legacy installation lacking the manifest/protocol.

## activation failure diagnostics

Automatic activation command stdout/stderr use the job's existing PTY attachment
and bounded, sanitized durable logs, rather than only the daemon service logs.
Independent operator switches retain output in their terminal/SSH session. The
original Switch error is retained in lifecycle logs and unresolved status.

A pre-activation failure is terminal only when the worker obtains the exclusive
activation lease, reloads a **Ready** receipt, and durably revokes it. The job
then fails (or is cancelled), FIFO can continue, and that receipt cannot switch
later. This is not an automatic activation retry. If revocation persistence
fails, execution/admission stop as Interrupted rather than trusting a possibly
non-durable receipt. A busy lease, absent/corrupt receipt, or attempted activation
(Activating/Uncertain) still blocks FIFO for independent reconciliation; no failed
command, unchanged profile, or disconnected client alone proves no side effect.

## interruption, cancellation, and reconciliation

The durable receipt lock excludes concurrent operator processes. Repeating
`switch JOB` after `Verified` only re-verifies; it does not activate twice. After
`Activating`, timeout, failure, process death, lost SSH or uncertain status, it
**refuses to replay** the switch. Inspect the receipt and service state, then use:

```sh
coeus reconcile JOB --ssh laptop \
  --activation-directory /var/lib/coeus/state/.coeus-state.activation \
  --peer-config /private/etc/coeus/server.toml
```

Reconcile is **observation only** and unblocks FIFO only when the exact closure and
authenticated expected daemon are actually present. A failed verification stays
blocked. It does not assume failure means no host change, roll back automatically,
or retry a partially applied system activation. If the target is not the requested
system, reconcile it in an independent operator session; never edit a receipt to
pretend a switch succeeded. After a lost connection inspect possible privileged
activation descendants as part of that operator reconciliation; SSH/process-group
cleanup alone cannot prove a root subprocess was terminated.

Cancellation while Ready revokes activation under the exclusive receipt lease and
releases FIFO only after the receipt file **and its directory** are synced. This
also applies when cancellation lands after receipt preparation while the job still
says Build, and to build-only/dry-run revocation. Failures are not classified as
pre-activation from the ledger phase alone: the worker leases and reloads the
receipt (or confirms no receipt/pending write exists before activation intent).
Busy leases, corrupt/missing expected receipts and uncertain states retain the
barrier; the daemon never replays activation.

A failed receipt save/fsync stops execution and new admission, recording
Interrupted when the ledger is writable. A readable Cancelled receipt after a
failed rename/directory-sync sequence is **not** enough to release FIFO. Normal
observers acquire the lease, reload and durably re-save terminal receipt state
before releasing the barrier. A persistence error is not retried by that worker;
restart retains Interrupted work for operator reconciliation. Once a
switch owns the receipt, cancellation is **not rollback** and cannot release the
barrier. During intentional daemon replacement, the new daemon resumes the passive
handoff observer and only the FIFO jobs durably known never to have started. Every
executor first claims Checkout durably. An unrelated crash still marks unfinished
work Interrupted and never replays it. Corrupt/inconsistent state fails closed.

Raw terminal replay remains volatile across daemon restart, as before. Build logs,
request IDs, phases and final cause remain durable. The independent activation's
command output goes to the operator SSH session; its durable record is the receipt,
not a fabricated daemon-owned PTY session.

## validation boundary

Automated acceptance uses injected build/activation/SSH/service-health executors,
throwaway ledgers and loopback encrypted transports only. No real SSH, activation,
Nix profile, launchd/systemd daemon or laptop configuration is changed by tests.
Source/module evaluation and Linux tests are not native Darwin activation proof.
Protocol **v5** introduced the changed Noise prologue and authenticated identity
requests. Current protocol **v6** adds authenticated hello, fleet machine status,
shared cross-job output and server-assigned IDs with separate retry nonces; upgrade
both client and daemon together. See [fleet client](fleet-client.md) for configuration
migration and target-aware commands. Historical on-disk phase variants were appended;
ledger migration preserves retained history rather than deleting it to unblock
admission.

### Explicit target hostname

Supported service configurations must set `networking.hostName` explicitly to a
simple 1–127 character hostname (letters, digits, hyphen or underscore). In
particular, nix-darwin's default `null` is not a target identity and is rejected
with a module assertion before installation; Coeus never guesses it from an SSH
alias or emits a manifest with a missing host.
