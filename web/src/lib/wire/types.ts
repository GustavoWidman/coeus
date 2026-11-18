// generated from src/web/view.rs by `COEUS_BLESS=1 cargo test web::`; change the rust types, not this file

/** epoch milliseconds */
export type Ms = number;
export type Hostname = string & { readonly __host: unique symbol };
/** 16 hex digits */
export type OrderId = string & { readonly __order: unique symbol };

/** one message of the fleet stream. every entity travels whole, so a lost or skipped message is repaired by the next one about the same entity */
export type FleetEvent =
  | { kind: "snapshot" } & Snapshot
  | { kind: "machine"; machine: MachineCard }
  | { kind: "progress"; progress: LiveProgress }
  | { kind: "progress_cleared"; host: Hostname }
  | { kind: "deployment"; deployment: DeploymentView }
  | { kind: "deployment_removed"; order: OrderId }
  | { kind: "expired" };

/** what a browser receives first, and again whenever it cannot resume */
export type Snapshot = {
  schema: number;
  /** the web app build this server serves */
  build: string;
  served_by: Hostname;
  viewer: Viewer;
  /** in inventory order */
  machines: Array<MachineCard>;
  progress: Array<LiveProgress>;
  /** newest first */
  deployments: Array<DeploymentView>;
};

/** who is signed in, on which device, until when */
export type Viewer = {
  login: string;
  device: string;
  expires_at: Ms;
};

/** `/api/hello`, the one thing under `/api` that answers before signing in */
export type Hello = {
  /** the serving hosts' sign-in addresses, reachable only on the tailnet */
  signin: Array<string>;
  /** the web app build this server serves */
  build: string;
  schema: number;
};

/** what a browser sends to sign in: its public key, and its signature over `coeus-signin-v1\n<public origin>\n<ts>` by the matching private key */
export type SignIn = {
  key: Jwk;
  ts: Ms;
  /** `r || s`, base64url, as WebCrypto signs */
  sig: string;
};

/** a public key as a JSON Web Key */
export type Jwk = {
  kty: string;
  crv: string;
  x: string;
  y: string;
};

/** what signing in on the tailnet answers: the session to send as `Authorization: Coeus <token>`, and who it is for */
export type SignedIn = {
  token: string;
  viewer: Viewer;
};

/** a machine, all but its running job's progress */
export type MachineCard = {
  host: Hostname;
  link: LinkView;
  /** what the machine last reported, kept while it is out of reach; unset until it is first reached, since only the machine knows its platform */
  state: MachineState | null;
};

/** a machine as its daemon last described it */
export type MachineState = {
  seen_at: Ms;
  platform: PlatformView;
  /** `aarch64-darwin`, `x86_64-linux`, ... */
  system: string;
  /** the daemon's coeus version */
  version: string;
  daemon: DaemonStateView;
  generation: number | null;
  /** `/run/current-system` */
  current: string | null;
  pending_boot: boolean;
  reboot_required: boolean;
  /** the source of the job that produced the running system, if one did */
  deployed: SourceView | null;
  follow: FollowView | null;
  running: JobCard | null;
  next: JobCard | null;
  /** finished jobs, newest first */
  recent: Array<JobCard>;
};

export type PlatformView = "nixos" | "darwin";

export type DaemonStateView = "serving" | "restarting";

export type LinkView =
  | { kind: "connected" }
  | { kind: "connecting" }
  | { kind: "unreachable"; since: Ms; reason: UnreachView };

export type UnreachView =
  | { kind: "offline"; last_seen: Ms | null }
  | { kind: "refused"; detail: string }
  | { kind: "failed"; detail: string };

/** the ref a machine catches up with on its own */
export type FollowView = {
  git_ref: string;
  /** where the ref pointed when the machine last looked */
  latest: PinnedView | null;
  paused: boolean;
};

export type PinnedView = {
  rev: string;
  committed_at: Ms;
};

export type JobCard = {
  id: number;
  /** 16 hex digits: an order id exceeds what a javascript number holds */
  order: OrderId;
  source: SourceView;
  action: ActionView;
  origin: OriginView;
  state: JobStateView;
  ordered_at: Ms;
  started_at: Ms | null;
  finished_at: Ms | null;
  /** when the running job entered its phase */
  phase_started_at: Ms | null;
  toplevel: string | null;
  attempt: number;
  /** how many packages the running job changes, once its closure diff is known */
  changes: number | null;
};

export type SourceView =
  | { kind: "git"; rev: string; git_ref: string | null }
  | { kind: "snapshot"; label: string; nar_hash: string }
  | { kind: "generation"; number: number };

export type OriginView =
  | { kind: "user"; user: string; host: string }
  | { kind: "follow" }
  | { kind: "github"; repository: string; workflow: string; run_id: number; attempt: number; url: string };

export type ActionView = "switch" | "boot" | "test" | "build";

export type PhaseView =
  | "preparing"
  | "fetching"
  | "evaluating"
  | "building"
  | "activating"
  | "verifying";

export type JobStateView =
  | { kind: "queued" }
  | { kind: "running"; phase: PhaseView }
  | { kind: "finished"; outcome: OutcomeView };

/** how a job ended. a failure's last lines are left out: the log view shows the end of the log itself */
export type OutcomeView =
  | { kind: "succeeded" }
  | { kind: "up_to_date" }
  | { kind: "cancelled" }
  | { kind: "interrupted" }
  | { kind: "superseded"; by: number }
  | { kind: "failed"; phase: PhaseView; message: string }
  | { kind: "rolled_back"; phase: PhaseView; message: string };

/** the running job's numbers, the only entity that changes several times a second */
export type LiveProgress = {
  host: Hostname;
  job: number;
  builds: TallyView;
  downloads: TallyView;
  downloaded_bytes: number;
  expected_bytes: number;
  /** the derivations building right now, by name */
  active: Array<string>;
};

export type TallyView = {
  done: number;
  expected: number;
  running: number;
  failed: number;
};

/** one order across the fleet, put together from the jobs machines report for it */
export type DeploymentView = {
  order: OrderId;
  source: SourceView;
  origin: OriginView;
  action: ActionView;
  /** the commit's subject; no machine reports it yet */
  subject: string | null;
  ordered_at: Ms;
  /** when the last ordered job finished */
  settled_at: Ms | null;
  waves: Array<WaveView>;
  /** ordered machines wave by wave, then the machines that follow the ref */
  rows: Array<DeploymentRow>;
};

/** machines ordered together */
export type WaveView = {
  /** what the deploy called this wave; no machine reports it yet */
  name: string | null;
  hosts: Array<Hostname>;
  started_at: Ms;
  settled_at: Ms | null;
};

export type DeploymentRow = {
  host: Hostname;
  /** index into the deployment's waves; unset for a machine that follows the ref instead of being ordered */
  wave: number | null;
  verdict: Verdict;
  job: number | null;
  diff: DiffSummary | null;
};

export type Verdict =
  | "waiting"
  | "running"
  | "done"
  | "failed"
  | "pending"
  | "catching_up"
  | "caught_up";

export type DiffSummary = {
  changes: number;
  /** net closure growth in bytes */
  size_delta: number;
};

export type DiffView = {
  changes: Array<PackageChangeView>;
};

export type PackageChangeView = {
  name: string;
  /** empty when the package is new */
  before: Array<string>;
  /** empty when the package is gone */
  after: Array<string>;
  size_delta: number;
};

/** one message of a job's log stream */
export type LogEvent =
  | { kind: "started"; job: JobCard }
  | {
      kind: "lines";
      lines: Array<LogLine>;
      /** the daemon's log offset after these lines, where a reconnect resumes */
      end: number;
    }
  | { kind: "ended"; outcome: OutcomeView }
  | { kind: "expired" };

export type LogLine = {
  /** when coeus recorded it; nix's own lines carry no time */
  at: Ms | null;
  kind: LineKind;
  text: string;
  /** the derivation whose builder printed it */
  prefix: string | null;
};

export type LineKind =
  | "coeus"
  | "phase"
  | "fetch"
  | "build"
  | "output"
  | "error"
  | "done"
  | "note";
