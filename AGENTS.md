# AGENTS.md

## Project standard

Coeus is a small Rust control plane for requesting deployments on a remote host. The
repository is intentionally split into narrow modules. Keep that quality bar as the
project grows: prefer clear ownership, short readable functions, stable interfaces,
and focused changes over clever abstractions or broad rewrites.

This file is repository guidance for contributors and coding agents. It describes the
architecture that exists today; update it when a deliberate architectural change makes
any section inaccurate.

## Repository map

- `Cargo.toml` / `Cargo.lock` — Rust package metadata and locked dependencies.
- `flake.nix` / `flake.lock` — Nix development shell, Crane package and service-module exports.
- `nix/modules/` — reusable non-root NixOS/systemd and nix-darwin/launchd services.
- `nix/elevation.py` — strict immutable helper policy for passwordless nh activation.
- `nix/tests/` — real module evaluation and unprivileged elevation-policy checks.
- `rust-toolchain.toml` — pinned nightly toolchain and required components.
- `.envrc` — loads the flake for direnv users.
- `config.client.default.toml` — embedded client defaults.
- `config.server.default.toml` — embedded server defaults.
- `src/bin/coeus.rs` — client executable entry point.
- `src/bin/coeusd.rs` — daemon executable entry point.
- `src/service_startup.rs` — native module-managed ownership/permission checks (Darwin).
- `src/cli/` — clap argument definitions for the two binaries.
- `src/client/` — target resolution, authenticated connections, fleet dispatch/status,
  raw cross-job/per-job attachment and fleet-only TUI.
- `src/server/` — listener, connection handling, and packet dispatch.
- `src/common/proto/` — protocol version, packet types, envelopes, and wire tests.
- `src/common/stream/` — Noise-encrypted, length-delimited stream and I/O adapters.
- `src/config/` — client/server configuration loading and defaults.
- `src/activation/` — independent exact-closure SSH/bootstrap activation and private receipts.
- `src/deploy/` — repository checkout and deployment command orchestration.
- `src/jobs/` — bounded FIFO admission, execution ownership and bounded durable job/log ledger.
- `src/utils/` — logging and PSK serialization helpers.
- `proto-macros/` — the procedural macro that generates packet serialization code.

Keep new code in the narrowest existing module that owns its behavior. Add a new
module or submodule when a file starts to own multiple independent responsibilities.
Do not turn `src/server/mod.rs`, `src/deploy/mod.rs`, or the stream module into catch-all
files merely because they are already public entry points.

## Runtime flow

### Client

1. `coeus` parses `MainCLIArgs`.
2. `config::client_config` loads the explicit file or XDG `coeus/config.toml`; it
   never generates/replaces credentials. Client settings contain key, fleet and
   default_revision. Config errors suppress secret-bearing source text.
3. Targets resolve reserved aliases, local OpenSSH configuration then DNS.
   `CoeusClient::new` opens a TCP connection and performs the Noise
   `Noise_NNpsk0_25519_ChaChaPoly_BLAKE2s` handshake.
4. Authenticated hello precedes requests; fleet dispatch is bounded to eight peers.
   `client::run` uses `CoeusClient::request` to concurrently send an envelope and
   receive one response. Cancellation/errors poison the connection against reuse;
   no deployment is automatically retried after an ambiguous acknowledgement.

### Server

1. `coeusd` parses `DaemonCLIArgs`; Darwin's module launches the native binary directly
   and requests in-process service ownership/permission checks before loading
   `ServerConfig`. Linux retains its existing service startup wrapper.
2. `CoeusServer::new` binds an `EncryptedListener` and constructs a `Deployer`.
3. `run` accepts TCP sockets cancellation-safely. Each tracked Tokio task owns a
   ten-second Noise handshake followed by its authenticated session; both pending and
   established sessions share a 64-task cap. Shutdown aborts and joins those tasks.
4. `handle_connection` receives packets in order and delegates them to
   `handle_packet`.
5. Deploy requests enter `jobs::DeploymentJobs`, a bounded FIFO queue; acknowledgement
   returns a job handle without waiting for earlier deployments or client attachment.
6. One daemon-owned worker executes checkout/build/switch. Status, bounded cursor logs,
   and cancellation requests are handled independently through response packets.
7. Build/switch output is merged on an output-only Unix PTY. Bounded volatile raw-byte
   pages feed `attach`; compact sanitized `terminal` records feed durable logs. No
   attachment owns deployment lifetime or forwards input/SIGINT.

`src/jobs/` owns admission, lifecycle and the versioned durable ledger. Ordering is
admission order under its FIFO Tokio mutex, not network arrival time. File/rename/
directory sync precedes ACK and executable queue insertion. Writes own their tasks:
disconnecting/cancelling an await must not reorder snapshots. One private state-directory
lease excludes another daemon. Restart interrupts started work, preserves never-started FIFO behind uncertainty,
and restores passive observers for Interrupted/manual barriers. Terminal receipt
settlement durably updates the ledger before FIFO/admission resumes. No started
job is replayed; missing/corrupt/nonterminal receipts remain blocked. Durable IDs
are not silently evicted. Git remains joined `spawn_blocking` with cooperative transfer
cancellation, not a hard checkout deadline. Child commands own Unix process groups.
See `docs/fleet-client.md` and `docs/async-deployments.md` for crash windows, storage bounds and remaining gates.

## Protocol and stream invariants

Treat the wire format as a compatibility boundary, not an implementation detail.

- Protocol v6 adds authenticated hello, machine status, shared cross-job output and
  server-assigned IDs with separate retry nonces. v5 introduced identity/manual-switch; v4 raw terminal pages. The Noise prologue versions the session; both peers
  must upgrade together. `PROTOCOL_VERSION` is carried by `PacketEnvelope`; reject unsupported versions.
- Packet IDs in `src/common/proto/mod.rs` are stable protocol identifiers. Never reuse
  an old ID for a different meaning.
- Add packet payload structs under `src/common/proto/types/`, register the variant in
  `Packet`, and add serialization and behavior tests in the protocol/stream test
  modules.
- Preserve the generated packet shape expected by `proto-macros`: the public enum
  declares unit variants with `#[packet(id = ..., type = ...)]`; the macro turns them
  into boxed payload variants.
- When wire compatibility matters, test both a round trip and the exact encoded bytes.
  Keep packet field ordering and postcard representation changes deliberate.
- `EncryptedStream` uses a Noise PSK handshake followed by u16 length-delimited frames.
  Respect `MAX_FRAME` and the 16-byte transport tag when adding framing logic.
- `EncryptedStream` also implements `AsyncRead` and `AsyncWrite`. Keep packet helpers
  and byte-stream behavior consistent, including EOF and partial-read behavior.
- Test encrypted behavior with loopback sockets and bounded timeouts. A network test
  must not be able to hang indefinitely on a handshake or missing peer.

## Rust design and implementation rules

- Follow Rust 2024 conventions and run rustfmt rather than hand-formatting code.
- Prefer small functions with one reason to change. Extract a named helper when it
  clarifies a protocol step, error boundary, or resource lifetime.
- Keep public APIs minimal. Make helpers private unless another module genuinely needs
  them; re-export only deliberate crate-level API.
- Use domain names (`DeployRequest`, `PacketEnvelope`, `CommandRunner`) instead of
  vague names such as `Manager`, `Data`, or `Helper`.
- Prefer typed values and structured arguments over strings. In particular, preserve
  `SocketAddr`, fixed-size `[u8; 32]` PSKs, `Path`/`PathBuf`, and `OsString` at their
  boundaries.
- Propagate errors with `Result` and `?`. Add useful context at subsystem boundaries;
  do not replace actionable errors with generic strings.
- Do not add `unwrap`/`expect` to runtime paths. A compile-time embedded default that
  is invalid is a different invariant from user or network input; make that distinction
  explicit when it is unavoidable.
- Do not add blocking filesystem, Git, or process work to async functions without an
  explicit blocking boundary.
- Do not log PSKs, credentials, or other secrets. Be careful when changing config
  debug output, error formatting, authentication callbacks, or deployment logging.
- Avoid unnecessary cloning, allocation, and synchronization, but do not sacrifice
  ownership clarity for micro-optimizations.
- Avoid `unsafe`, global mutable state, and new dependencies unless the need is clear,
  documented, and reviewed.
- Keep files cohesive and readable. Split a growing file by responsibility rather
  than allowing a large `mod.rs` or one oversized function to accumulate unrelated
  behavior.
- Match the surrounding style before introducing a new pattern. Consistency is more
  valuable than adding a framework for a single call site.

## Configuration and security

- Runtime `config.client.toml`, `config.server.toml`, and `config.toml` files are
  ignored. Do not commit machine-specific configuration or real PSKs.
- The tracked `*.default.toml` files are compile-time defaults and are safe only as
  development examples. A production deployment must provide an explicit matching PSK
  on both peers.
- Preserve the custom 64-character hexadecimal PSK serializer/deserializer contract.
  Invalid length, non-ASCII input, and non-hex input must remain rejected.
- Treat repository URLs, revisions, and deployment flags as untrusted input. Keep
  Git credentials scoped to the temporary checkout and preserve argument separation
  when invoking `nh`.
- Do not weaken Noise parameters, framing validation, revision checkout, or credential
  handling to make a test pass. Add a focused test and document the security tradeoff
  if a protocol or deployment change is required.

## Tests and validation

Run commands from the repository root. The intended environment is the pinned Nix
shell (`nix develop`) or a direnv shell loaded from `.envrc`.

```sh
# Formatting
cargo fmt --all -- --check

# Unit and integration tests
cargo test --all-targets

# Linting; warnings are treated as failures for reviewed changes
cargo clippy --all-targets --all-features -- -D warnings

# Package/build validation when the change affects binaries or packaging
cargo build --all-targets
nix build

# Review hygiene
git diff --check
git status --short
```

Run the narrowest relevant test while iterating, then run the complete suite before
finishing. Changes to protocol or stream code require the packet and encrypted-stream
tests at minimum. Changes to deployment or command execution require the relevant
command/deployer tests and a review of blocking, cleanup, and argument handling.

The project currently has no checked-in CI workflow; local formatting, tests, clippy,
and build checks are the quality gate. The flake pins the Rust toolchain to
`nightly-2026-09-08` and includes `rustfmt`, `clippy`, `rust-src`, and `rust-analyzer`.
The dependency graph enables vendored OpenSSL through `git2`, so native builds need a
C toolchain with `make` and `perl`. If `openssl-sys` reports `make` missing, fix or
extend the development shell rather than changing dependency features just to bypass
the check.

The baseline's obsolete `deploy::utils::ssh_string` has been removed; do not reintroduce
unused insecure SSH helpers or broad warning suppressions. Native builds require
make/perl for vendored OpenSSL. The flake now supplies these tools and includes the
embedded default TOML files in its filtered package source.

## Change and commit workflow

1. Start with `git status --short --branch` and inspect recent history.
2. State the intended behavior change and the smallest affected module set before
   editing.
3. Make one focused change. Do not mix refactors, formatting churn, generated files,
   dependency upgrades, and behavior changes in one patch.
4. Add or update tests with the change. Prefer behavior-level tests over tests coupled
   to private implementation details.
5. Run formatting, focused tests, then the complete validation commands above.
6. Review `git diff --stat`, `git diff`, and `git diff --check`. Confirm no secrets,
   temporary files, build artifacts, or unrelated edits are present.
7. Commit one logical change at a time. Existing history uses concise Conventional
   Commit subjects such as `feat(server): acknowledge deploy requests`,
   `fix(deploy): run commands from checkout directory`, and `build: pin Nix to the
   Rust toolchain file`. Follow the same `<type>(<scope>): <imperative summary>` shape
   when a scope adds clarity.

Do not rewrite or squash unrelated history. Do not push unless explicitly requested.

Service setup and the root-equivalent activation boundary are documented in
`docs/nix-services.md`. Do not run a host activation just to validate modules;
use the flake checks. Daemon-changing builds must use the independent manual-switch flow; see `docs/independent-activation.md`. Native platform activation still requires explicit operator authorization and separate runtime validation.
