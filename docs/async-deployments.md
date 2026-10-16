# async deployment milestone

Local work on top of the still-open stream-halves PR #1. This does not merge the
PR, install a daemon, run a deployment, or integrate GitHub Actions. Both peers
must use protocol **v6** (including its Noise prologue); older peers fail the
handshake before they can submit an incompatible request.

## ownership and acknowledgement

`DeploymentJobs` owns one FIFO executor and a bounded channel (16 queued plus one
running). Order is admission order under the ledger lock, not TCP arrival order.
The daemon, not an attached CLI connection, owns accepted execution. Dropping an
admission/cancellation/log-write future cannot interrupt its disk transaction:
owned tasks retain the ledger lock through persistence and subsequent queue or
cancellation notification. An older cancelled log write cannot overwrite a newer
terminal snapshot.

Production admission is durable. The caller supplies a nonzero 64-bit **retry
nonce**, separate from the daemon-assigned ascending job handle. Before sending,
the CLI flushes target + nonce; `--request-id` reuses it with identical options
on one explicit target after a reviewed lost acknowledgement. Identical retries
return the original handle, conflicting reuse fails. The daemon persists its
allocation high-water mark before an ID can be acknowledged; failed admission may
leave gaps. A restart must never reuse an acknowledged ID. Legacy durable IDs remain
retained, and migration starts beyond their maximum rather than renumbering jobs.
This is not exactly-once host side effects. Never generate a new nonce to recover a
lost ACK. Idempotency is scoped to one machine's state and configured repository URL;
peers sharing its PSK share this namespace.

Each record is serialized to a fixed private pending file, fsynced, atomically
renamed to `<job-id>.job`, then the directory is fsynced. Only then can the
request enter the executable queue or return an ACK. Phase changes are persisted
before checkout/build/switch; resolved commit, cancellation intent, bounded log
tail, and terminal outcomes are persisted too. Cancellation intent reaches disk
before the executor is signalled. Disk errors stop admission/execution, cancel
contexts and expose an uncertain `Interrupted` status instead of claiming success.
Reads remain available for diagnosis. A write error can occur after rename:
reconnect/restart with the **same retry nonce**, never assume failure means no admission.

## CLI interfaces

See [fleet client](fleet-client.md) for configuration, resolver, authentication,
interactive dispatch and bootstrap behavior.

```sh
# Examples only; do not run against a production daemon during development.
coeus deploy lab --rev <full-sha> --request-id <retry-nonce> --no-tty
coeus status lab
coeus status lab --job <job-id> --watch
coeus attach lab
coeus logs <job-id> --target lab --after <cursor> --follow
coeus cancel <job-id> --target lab
```

Client TCP connection has a three-second per-address deadline; the Noise handshake
has ten seconds, then the authenticated hello/request has fifteen seconds.
An interrupted/partial request poisons its connection. No automatic deploy retry
occurs. The CLI reports
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

Target/nonce, acceptance and requested status tables remain visible even at quiet
verbosity. Durable `logs` are compact sanitized text; `--metadata` includes cursor/
source. New PTY output is `terminal`, not separately stdout/stderr. `attach` writes
raw terminal bytes without logger decoration. Diagnostics and lifecycle logging
remain distinct from the byte stream.

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

Before admission, v2 initializes and fsyncs its monotonic `id-counter`, then its
`ledger-format` marker. Restart rejects a missing counter when the marker, a
pending initialization file, or any v2 snapshot exists; it never reconstructs a
possibly pruned high-water mark. Malformed counters/markers and counters below
retained handles also fail closed. A genuinely new empty ledger and a v1-only
legacy ledger without initialization evidence can initialize (legacy migration
starts at its largest retained random handle, without rewriting terminal history).
The marker survives an empty/pruned ledger and burned reservations. Removing
**all** counter, marker/pending files and snapshots is indistinguishable from an
explicit fresh-state wipe and destroys this protection; the counter is not a
cryptographic anti-tamper/rollback mechanism.

At most **128 records**, **256 chunks/job**, and **1,024 UTF-8 bytes/chunk** are
retained. Each snapshot is capped at 300 KiB, plus one same-size pending file and
a lock file, an eight-byte counter and a small initialization marker (with bounded
pending writes): under 39 MiB logical data. Admission refuses when durable history is
full; it does not evict idempotency keys. There is **no automatic archival or safe
ID-expiry policy yet**. Manually deleting/resetting state discards retry protection
and is not a normal recovery operation. The explicitly volatile constructor used
by small tests remains separate: it evicts only finished records and their raw
terminal histories together, under ledger-then-terminal-map lock order. Retained
and active/queued records keep their histories. Durable admission never evicts
records or their retry IDs; restart alone discards its volatile raw output.

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
subscriber broadcasts. Old chunks are evicted with an explicit gap. Finished-job
log pages must still be drained. Build/switch stdout and stderr share one Unix PTY;
the observed bytes are ordered merged output, so original descriptor identity is
lost. New durable records use `terminal`, while existing historical stdout/stderr
records remain readable and filterable. Long unterminated lines use bounded chunks
on complete UTF-8 boundaries (invalid/incomplete UTF-8 becomes replacement
characters). ANSI CSI, OSC, DCS and other escape payloads are consumed across PTY
reads without keeping their payload in memory. Tabs and useful plain text survive;
other terminal/directional controls do not. CR-only progress redraws coalesce until
newline/EOF rather than generating a record on every tick; CRLF is one line break.
A partial line is emitted at EOF or the size bound, not on a timer. An unterminated
escape string is discarded through EOF (OSC accepts BEL or ST; other strings
require ST); malformed non-string escapes recover at a line break. Cancellation
may drop an unfinished line, but in-flight durable writes still finish.

`logs --follow` is compact plain text, **not** the terminal view. `attach` is the
separate output-only raw PTY view with ANSI colors, cursor movement and progress
redraws. Commands get `TERM=xterm-256color` and initial 80×24 geometry; attached
views update the window size, but never forward stdin or SIGINT. See the terminal
attachment section for lifetime, replay and terminal-side-effect limits.
Lifecycle/status and client-side log text also remove complete control sequences;
this is defense in depth, not a replacement for authenticated peers. Raw `attach`
output is deliberately not sanitized. Existing durable history is not rewritten.
Old ESC-less fragments such as `[32m` cannot be reliably distinguished from literal
text, so remain visible; original CR updates/UTF-8 already lost cannot be recovered.
Status listings use 32-job pages, not a cross-page snapshot. Arbitrary secrets in
command output are **not** scrubbed. Treat private state/logs as sensitive.

Every retained chunk currently fsyncs its bounded per-job snapshot on a blocking
pool, under the FIFO ledger lock. This deliberately favors simple lifecycle
correctness over write throughput: storage latency backpressures child output and
can delay status/ACKs. It does not grow an unbounded write queue. Group commit or a
transactional log store is a later, measured optimization; no throughput gain is
claimed. On cancellation, an in-flight write finishes before terminal persistence.

For daemon-owned exact-closure activation, the fixed privileged helper now supports
an explicit supervised mode. The validated activation command runs in a helper-owned
process group; a daemon-held stdin lease keeps it alive. Closing the lease on local
cancel/timeout allows the privileged owner to terminate/reap descendants instead of
asking the unprivileged daemon to kill root children. The command/closure allowlist
and scoped sudo rule are unchanged. Fake tests exercise this process-tree lifetime
without root; this is not proof against arbitrary deliberately escaping code or a
real Darwin FDA denial. Both automatic and independent scoped-helper activation
own their deadline and join lease cleanup; no outer timeout/cancellation may drop
that future. The activation budget is 900 seconds across commands; cleanup can
extend it by up to five seconds. Post-switch verification checks job cancellation
between bounded probes. Bootstrap/operator-sudo has no preinstalled supervisor:
cleanup there can lack authority over root descendants and remains uncertain,
never a claim of root-tree termination.

A cancellation after activation intent can settle as terminal **Interrupted** with
cancel_requested=true, not falsely Cancelled/rolled back. Its original uncertain
receipt remains; subsequent FIFO/admission stay blocked, including across restart,
until independently reconciled. A missing/busy/unreadable receipt also fails closed;
the passive manual observer stops boundedly on cancellation rather than waiting for
permission forever. Do not delete state or retry a switch to bypass uncertainty.

Cancellation is a request, **not rollback**. Queued jobs are skipped. Ordinary
commands have null stdin and a one-hour deadline. Supervised activation instead
has a daemon-held pipe used only as a lifetime lease, never for interactive input.
Commands own Unix process groups; normal error/cancellation paths explicitly await
cleanup. Drop/unwind makes a best-effort kill request, not an asynchronous joined
cleanup guarantee. Linux and macOS are intended platforms; terminal-backed command
execution is unsupported on non-Unix systems. Deliberately escaped children are not
sandboxed. A cancelled/failed switch may already have modified the host. Exact-closure
activation verifies the expected closure and authenticated daemon, but this is not
a general application-health check or proof that every host side effect is correct.

Checkout remains joined `spawn_blocking` libgit2. Cooperative fetch-progress
callbacks stop transfers when cancellation is observed; checks bracket clone and
checkout. This is **not a hard deadline**: stalled DNS/SSH/credential calls or a
checkout without progress callbacks can delay cancellation and graceful shutdown.
SIGINT/SIGTERM close admission/connections and cancel work. Forced kill differs.
`--dry-run` is build-only and never activates. See
[independent activation](independent-activation.md) for retained-root semantics.

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
handshake. The OS TCP backlog is separate and not an authentication guarantee.
No interactive input/prompt forwarding, GitHub Actions
integration, reconciliation, archival, build-only/boot policy, or Darwin deployment
parity is claimed.

After independent review of the exact frozen tree: a multi-machine **fake-executor**
trial with separate private ledgers/test PSKs, deterministic IDs, dropped ACKs,
concurrent status/log attachment, saturation, disconnect/restart and cancellation.
Measure ACK/status latency and persistence volume under equivalent fake output.
Never invoke `CoeusServer::new`/`Deployer` or real `nh` in that trial. Existing tests
use injected executors, ephemeral loopback ports, local Git fixtures and harmless
shell fixtures only; no real credentials, host switches, services or infra.

## Terminal attachment (introduced in v4; current protocol v6)

`coeus attach TARGET` is a **cross-job output-only terminal view**, separate from durable logs.
Build/switch commands run with stdout and stderr on the same Unix PTY, with
`TERM=xterm-256color`, initially 80 columns × 24 rows. Both descriptors see a TTY;
ordinary command stdin is closed (`/dev/null`); supervised activation uses its
private lifetime-lease pipe instead. The output PTY is not installed as a controlling
terminal, and no client input is forwarded. The daemon session is unchanged: if
launched from a terminal, commands may still inherit that existing controlling TTY.
Run an unattended daemon without relying on prompts on that TTY.
Interactive password/confirmation prompts are not supported; configure unattended
credentials/elevation before submitting a real deployment. Existing `nh` flags,
attachment does not own deployment lifetime. Exact-closure activation and build-only
`--dry-run` semantics are documented in [independent activation](independent-activation.md).

```sh
coeus deploy lab --rev main --no-tty
coeus attach lab                    # daemon-global replay, then future jobs
coeus attach lab --live             # explicitly skip replay
coeus status lab --watch
coeus logs 1 --target lab --follow
coeus logs 1 --target lab --metadata
coeus logs 1 --target lab --source terminal
coeus cancel 1 --target lab          # only this command cancels the job
```

Attach writes raw ordered bytes without logger prefixes, Unicode conversion or
newlines. ANSI colors, cursor movement and CR progress redraws reach the terminal
unchanged. Local terminal dimensions are sampled while attached and applied to the
PTY; resizing sends SIGWINCH to the owned command group, never SIGINT. If several
views attach, the most recent requested dimensions win. Slow/disconnected viewers
cannot block the daemon-owned job. Ctrl+C exits/detaches the local view only; it does
not forward input/signals, cancel the job or stop coeusd. The client never enters raw
input mode: only local output postprocessing is temporarily disabled to avoid
doubling CR/LF. File flags and all termios fields are restored on normal/detach/error
exits; SIGKILL cannot run cleanup. On a local TTY it resets SGR styling and shows the cursor when exiting;
redirected stdout is byte-exact and receives no cleanup controls. Alternate-screen
or other application-specific modes are not comprehensively restored.

Raw replay is **volatile**: the shared cross-job feed is capped at 256 KiB, with
pages capped at 4096 bytes. The internal per-job replay API additionally retains
256 KiB per job (at most 128 jobs / 32 MiB) for automatic accepted-job follow.
There is no subscriber queue/backpressure tied to the deployment. `--after BYTE`
resumes at an exact absolute byte cursor; it is not a durable log cursor. If bytes
were evicted, attach stops with an explicit history-gap error instead of feeding a
partial escape/state replay to the terminal. `--live` skips history explicitly but
cannot reconstruct earlier screen contents/colors. Restart resets the shared feed and its cursor; an old out-of-range cursor fails.
The per-job replay API reports pre-restart bytes unavailable. `status` and sanitized
durable logs remain usable.
Unfinished mutations are Interrupted and never automatically replayed; a durable manual
handoff resumes only its passive observer and never-started FIFO jobs. Protocol v6
changes the Noise prologue and adds hello, active/latest status and shared-terminal
packets; upgrade both binaries. New binaries migrate legacy histories and retain
old stdout/stderr source IDs. Old binaries cannot read newly written `terminal`
source records, so downgrading a daemon with new history is not supported.

PTY stdout/stderr are physically merged, preserving kernel stream order but losing
original descriptor identity. New durable records therefore have source `terminal`;
`--source stderr` only selects truly stderr-labeled historical/pipe records, not PTY
output. Logs default to compact plain text; CR-only redraws are coalesced until a
newline/EOF rather than logged on every tick. Lifecycle lines remain visible. Long
lines still obey the existing 1024-byte chunk and retained-log bounds, so this is not
a full terminal screen recording. Raw terminal controls are intentionally **not
sanitized**: only attach to a trusted authenticated deployer/output producer. Colors,
OSC titles and other controls have the same terminal-side effects as running the
command directly; no claim of terminal-control or secret redaction is made.

The PTY implementation uses safe `nix::pty::openpty`, readiness-based nonblocking
master reads and safe `rustix` window ioctls, with no application-level unsafe or
blocking reader thread. Process groups remain daemon-owned and killed/reaped on
cancel/deadline/shutdown. Master/slave handles close with command/drain ownership;
view disconnects never own any of these resources. Hard daemon death may still leave
children, as documented above; attachment does not solve host reconciliation or
production readiness. Validation uses harmless real-PTY shell fixtures and encrypted
loopback, not a real `nh`/Mac switch.

## Interrupted activation recovery

Cancellation stops owned execution; it is not rollback. If an attempted activation
has an uncertain result, the job is `Interrupted` and the machine reports
`Recovery required`. New admission and queued effects remain blocked. The FIFO
worker stays alive as a passive receipt observer, including after daemon restart.
It never re-executes that job. Never-started queued jobs remain queued behind the
barrier instead of being lost during restart.

A compatible terminal receipt is observed under the activation lease and made
durable before the job ledger is settled. `Verified` becomes `Succeeded`, even if
cancellation was requested: the intended system state was verified, not undone.
`Cancelled` means the receipt was revoked before activation intent. Missing,
corrupt, busy, `Ready`, `Activating`, or `Uncertain` receipts cannot release an
Interrupted barrier. Every barrier must settle before later queued effects run.
Receipt or ledger persistence failure blocks the current daemon instance.

`coeus reconcile` is verification only. When it records a valid `Verified` receipt,
the observer updates the ledger and resumes FIFO without replay or a daemon
restart. It cannot resolve a failed partial activation that does not verify as the
new closure. Such a partial failure requires a separate explicit operator recovery
path; neither exit status nor process absence proves writer cleanup. Do not wipe
state or replay activation to remove the barrier.

An authenticated rejection is shown as a rejection, not a lost acknowledgement.
Transport loss still warns that submission might have been accepted. Always query
the target's status before another submission when persistence or transport is
uncertain.
