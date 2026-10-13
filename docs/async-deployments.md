# async deployment milestone

Local work on top of the still-open stream-halves PR #1. This does not merge the
PR, install a daemon, run a deployment, or integrate GitHub Actions. Both peers
must use protocol **v3** (including its Noise prologue); older peers fail the
handshake before they can submit an incompatible request.

## ownership and acknowledgement

`DeploymentJobs` owns one FIFO executor and a bounded channel (16 queued plus one
running). Order is admission order under the ledger lock, not TCP arrival order.
The daemon, not an attached CLI connection, owns accepted execution. Dropping an
admission/cancellation/log-write future cannot interrupt its disk transaction:
owned tasks retain the ledger lock through persistence and subsequent queue or
cancellation notification. An older cancelled log write cannot overwrite a newer
terminal snapshot.

Production admission is durable. The caller supplies a nonzero 64-bit request ID,
which is also the job handle. Before sending, the CLI logs and flushes the ID;
`--request-id` lets it be reused after a lost acknowledgement. The daemon stores
the full revision/flags and validates duplicate content before checking queue
capacity: identical retries return the original handle, conflicting reuse fails.
It does **not** promise exactly-once host side effects. Sending a different ID
creates a new submission, not a retry. Never generate a fresh ID to recover a lost
ACK. Idempotency is scoped to one machine's retained state directory and configured
repository URL. Peers sharing its PSK also share this namespace.

Each record is serialized to a fixed private pending file, fsynced, atomically
renamed to `<request-id>.job`, then the directory is fsynced. Only then can the
request enter the executable queue or return an ACK. Phase changes are persisted
before checkout/build/switch; resolved commit, cancellation intent, bounded log
tail, and terminal outcomes are persisted too. Cancellation intent reaches disk
before the executor is signalled. Disk errors stop admission/execution, cancel
contexts and expose an uncertain `Interrupted` status instead of claiming success.
Reads remain available for diagnosis. A write error can occur after rename:
reconnect/restart with the **same ID**, never assume failure means no admission.

## CLI interfaces

```sh
# Examples only; do not run against a production daemon during development.
coeus --config machine.client.toml deploy --rev <full-sha> --request-id <nonzero-id>
coeus --config machine.client.toml status <job-id>
coeus --config machine.client.toml status --watch
coeus --config machine.client.toml logs <job-id> --after <cursor> --follow
coeus --config machine.client.toml cancel <job-id>
```

Client TCP connection plus Noise handshake has a ten-second combined deadline;
a request/response has a fifteen-second deadline and poisons a partially used
connection on interruption. No automatic deploy retry occurs. The CLI reports
queued/checkout/build/switch/terminal **phases**, not invented Nix percentages.
A mutable `main` resolves during checkout, not admission: use a full SHA for
exact-revision semantics. The checkout's HEAD is detached at the resolved commit.
An exact 40-hex commit ID must already be a commit in the configured repository's
clone: publish it and make it reachable from advertised refs first. Local-only
commits or staged files on the requesting machine are not included. Missing IDs
fail with their original Git error and explicit repository-clone context; they
are never reinterpreted as `origin/<SHA>` branches or fetched separately. Named
revisions still resolve normally, with an exact remote-branch fallback for
non-default branches.

Human acceptance, status, lifecycle and gap-warning messages use the existing
logger, including its timestamps, levels and `--verbosity` filter. One explicit
safety exception: the pre-send `request-id` INFO message remains visible and
flushed even at `--verbosity off`, so a lost ACK cannot hide the retry key.
`logs` child stdout/stderr chunks are requested payload data: they retain their
cursor/source labels on stdout even at quiet verbosity, without a second logger
timestamp or level. This CLI has no machine-output mode; normal logger output
also goes to stdout.

## storage, bounds and restart

`settings.state_directory` defaults to `.coeus-state` relative to the daemon's
working directory; use an explicit persistent absolute path for an installed
service. Its parent must exist. Coeus creates the directory with mode 0700 and
files with 0600; it rejects public permissions, symlinks/nonregular files and
hard-linked files. A file lock protects against two live ledger owners. Local
same-user/root tampering is outside this protection; do not put the state on an
untrusted shared/network filesystem. Durability depends on the filesystem honoring
fsync/atomic rename. Snapshot format version, repository scope, filenames, field
bounds and log cursors are validated before startup. Incompatible/corrupt state
fails closed, not by silently resetting it. PSKs are not stored in snapshots.

At most **128 records**, **256 chunks/job**, and **1,024 UTF-8 bytes/chunk** are
retained. Each snapshot is capped at 300 KiB, plus one same-size pending file and
a lock file: under 39 MiB logical data. Admission refuses when durable history is
full; it does not evict idempotency keys. There is **no automatic archival or safe
ID-expiry policy yet**. Manually deleting/resetting state discards retry protection
and is not a normal recovery operation. The explicitly volatile constructor used
by small tests remains separate and may evict terminal records.

On startup, **every nonterminal record**, even Queued, is persisted as terminal
`Interrupted`; none is automatically executed. Logs and handles survive. Existing
terminal records remain terminal. A repeated ID still returns its retained handle,
not a new execution. If any record is Interrupted, all **new admissions are blocked
until operator reconciliation**. A previous child may still be alive, or the host
may already have switched. An operator-reconciliation/archival command is not yet
implemented; this fail-closed gate deliberately limits this milestone to fixtures.

Crash windows:

| Crash point | Recovery |
| --- | --- |
| Before snapshot rename | No admitted record; partial fixed pending file is ignored. No queue execution occurred. |
| After durable admission, before queue insertion/ACK | Retained ID becomes Interrupted; same-ID retry returns it without execution. |
| During checkout/build/switch | Last durable phase/log tail survives; job becomes Interrupted, not failed/successful by inference. |
| Host switched, terminal record not yet synced | Outcome stays uncertain; reconcile host before any new submission. |
| After terminal sync, before client observes it | Retained terminal result; same-ID retry/query does not execute again. |

A process-exit fixture verifies the Switch/restart window with fake execution;
it is not a power-cut test or proof of host recovery. Hard kill does not guarantee
child cleanup. Coeus does not inspect/reap an old process tree on restart.

## logs, cancellation and performance tradeoffs

Terminal lifecycle log entries include the bounded failure/cancellation cause,
not only the phase name, so attaching after a checkout/spawn failure is useful
even when no child output exists. Spawn errors identify the requested program,
without dumping arguments or the daemon environment.

Logs are cursor-paginated pull (32 chunks/page; 500ms follow), not unbounded
subscriber broadcasts. Old chunks are evicted with an explicit gap. Terminal log
pages must still be drained. stdout/stderr drain concurrently; their observed
ordering is not a total child-output ordering. Long unterminated lines use bounded
chunks. Status listings use 32-job pages, not a cross-page snapshot. Control
characters are stripped before retention; arbitrary secrets in command output are
**not** scrubbed. Treat private state/logs as sensitive.

Every retained chunk currently fsyncs its bounded per-job snapshot on a blocking
pool, under the FIFO ledger lock. This deliberately favors simple lifecycle
correctness over write throughput: storage latency backpressures child output and
can delay status/ACKs. It does not grow an unbounded write queue. Group commit or a
transactional log store is a later, measured optimization; no throughput gain is
claimed. On cancellation, an in-flight write finishes before terminal persistence.

Cancellation is a request, **not rollback**. Queued jobs are skipped. Commands
have null stdin, a one-hour deadline, and own Unix process groups killed/reaped on
error/cancellation/unwind. Non-Unix cleanup only targets the direct child; Linux
and macOS are intended platforms. Deliberately escaped children are not sandboxed.
A cancelled/failed switch may already have modified the host; command success
also does not prove remote health. Explicit post-switch verification is future work.

Checkout remains joined `spawn_blocking` libgit2. Cooperative fetch-progress
callbacks stop transfers when cancellation is observed; checks bracket clone and
checkout. This is **not a hard deadline**: stalled DNS/SSH/credential calls or a
checkout without progress callbacks can delay cancellation and graceful shutdown.
SIGINT/SIGTERM close admission/connections and cancel work. Forced kill differs.
The existing `--dry-run` flag only disables eval-cache and **still switches**; it
is not a simulation mode.

## security and next trial

The daemon owns the repository URL; clients provide only bounded revisions/flags.
No PSKs are dumped in config logging. A shared PSK grants deployment, cancellation,
and sensitive log access; per-user authorization, privilege policy, repository
policy, key provisioning and transport hardening remain production requirements.
Pending Noise handshakes and authenticated sessions share a 64-task cap. TCP
acceptance is cancellation-safe; each accepted socket owns its handshake and
authenticated session in one tracked task. A silent peer expires after ten seconds
without blocking other handshakes below the cap. Shutdown aborts and joins both
pending and authenticated tasks; completing another session cannot cancel a
handshake. The OS TCP backlog is separate and not an authentication guarantee. No TUI, multi-machine fan-out, GitHub Actions integration, reconciliation,
archival, build-only/boot policy, or Darwin deployment parity is claimed.

After independent review of the exact frozen tree: a multi-machine **fake-executor**
trial with separate private ledgers/test PSKs, deterministic IDs, dropped ACKs,
concurrent status/log attachment, saturation, disconnect/restart and cancellation.
Measure ACK/status latency and persistence volume under equivalent fake output.
Never invoke `CoeusServer::new`/`Deployer` or real `nh` in that trial. Existing tests
use injected executors, ephemeral loopback ports, local Git fixtures and harmless
shell fixtures only; no real credentials, host switches, services or infra.
