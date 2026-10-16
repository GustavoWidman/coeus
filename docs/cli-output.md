# Client output

Coeus has three deliberately different output paths:

- `deploy` prints the target and request ID before submitting, then the assigned
  job ID after an acknowledgement. `--no-tty` dispatches without following.
- Interactive fleet deployment uses a structured view. Single-target deployment
  follows the child's terminal output. Ctrl+C detaches either view, not the job.
- `attach TARGET` is the child's byte-exact terminal stream across jobs. It has no
  Coeus prefixes, color filtering, input forwarding or synthetic progress.
  `logs JOB --target TARGET --follow` reads compact durable lines instead.

## Fleet view

Fleet deployment groups accepted targets before dispatch errors, preserving
configuration order within each group regardless of response order. Status rows
follow configuration order. Wide terminals use aligned columns; narrow terminals
use target/job/state cards. Long cells end in `~`. Unicode cell width is accounted for; terminal controls from remote labels
and status text are removed. The final terminal column is reserved to avoid
automatic wrapping.

The header counts active observers, successful jobs and results needing attention.
These counts describe observations, not machine health or proof of a clean system.
There are no guessed percentages or completion times. Status is polled while a
job runs; unchanged screens are not repainted. Resize is detected without
restarting or cancelling a request.

Use `j`/`k` or arrow keys to scroll, Space to page, `g`/`G` for first/last lines.
`q` or Ctrl+C detaches. Scrolling does not send input to the deployment. If the
view loses its connection, the job may still be running. Query its status before
submitting another deployment. Terminal input/output modes, cursor visibility and
alternate screen are restored on exit and errors.

`status --watch` uses an alternate-screen view on capable terminals. Without a
capable terminal it emits only changed snapshots, not a full table every poll.
Run `status` without `--watch` for every retained row and full identifiers in a pipe.

## Colors and plain output

Cyan marks active work, green success, yellow waiting/cancelled, and red failure,
uncertainty or observation loss. Text labels always carry the meaning. Colors are
automatic per output stream, not forced into pipelines. `NO_COLOR` (including an
empty value), `CLICOLOR=0` and `TERM=dumb` disable semantic colors. A capable TTY
with `NO_COLOR` still gets a readable plain fleet view. `TERM=dumb` uses plain
dispatch output rather than cursor control. Raw child attachment remains
unchanged by this policy.

Normal client diagnostics go to stderr without timestamps or Rust source
locations. Debug/trace verbosity retains diagnostic detail. The mandatory target
and request ID remain on stdout even with logging disabled. Never put credential
contents in messages.

## Honest state and recovery wording

- A job acknowledgement gives a known job ID. Losing observation afterward does
  not undo its acceptance and does not stop execution.
- A lost, unexpected or invalid acknowledgement means acceptance is unknown.
- A generic authenticated `RequestRejected` means the daemon reported an error.
  It does **not** prove admission was never durably recorded: persistence may have
  failed after a rename. Its presentation therefore also retains acceptance
  uncertainty, while identifying the daemon refusal rather than a lost response.
- Connection/validation failure before sending means the request was not
  submitted. A fleet aggregate never turns an ambiguous target into a rejection.
- `Cancel requested` is not `Cancelled`. `Recovery required` and `Waiting for
  switch` block new work. Neither is described as a successful deployment.
- A failed last job is shown as failed, not machine-ready or healthy.
- Operator recovery records an explicit attestation about a partial activation.
  It is not proof of cleanup, verification, rollback, or successful deployment.
  It does not replay activation. See `independent-activation.md` for its guarded
  configuration-bound interface and prerequisites.

Generated operational text uses direct wording and ordinary punctuation. Coeus
must not rewrite a child's output or historical records to impose that rule.

## Validation boundaries

Compiled CLI tests use authenticated loopback peers and genuine PTYs. They cover
narrow, normal and wide rendering, resize, scrolling, semantic/plain color,
ordered targets, changed-only watch output, uncertainty and terminal restoration.
Separate tests preserve byte-exact raw attachment and cancellation separation.
These fixtures do not deploy a machine. Real Darwin service and privileged
activation/cancellation behavior require separate authorized runtime validation.
