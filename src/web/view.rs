use serde::Serialize;

use crate::common::model::{Hostname, Timestamp};

/// the version of these shapes; raised only when a change would break a
/// browser still running the previous build
pub const SCHEMA: u16 = 1;

/// what a browser receives first, and again whenever it cannot resume
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct Snapshot {
    pub schema: u16,
    /// the web app build this server serves
    pub build: String,
    pub served_by: Hostname,
    pub viewer: Viewer,
    /// in inventory order
    pub machines: Vec<MachineCard>,
    pub progress: Vec<LiveProgress>,
    /// newest first
    pub deployments: Vec<DeploymentView>,
}

/// who is signed in, on which device, until when
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct Viewer {
    pub login: String,
    pub device: String,
    pub expires_at: Timestamp,
}

/// `/api/hello`, the one thing under `/api` that answers before signing in
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct Hello {
    /// the serving hosts' sign-in addresses, reachable only on the tailnet
    pub signin: Vec<String>,
    /// the web app build this server serves
    pub build: String,
    pub schema: u16,
}

/// `/api/push`: the key a browser subscribes with, and the subscriptions
/// the fleet sends to, each the first 16 hex digits of its endpoint's
/// SHA-256
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct PushView {
    /// the VAPID public key, base64url, the browser's `applicationServerKey`
    pub key: String,
    pub subscribed: Vec<String>,
    /// the command that edits the subscription list
    pub edit: Option<String>,
}

/// what signing in on the tailnet answers: the session to send as
/// `Authorization: Coeus <token>`, and who it is for
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct SignedIn {
    pub token: String,
    pub viewer: Viewer,
}

/// one message of the fleet stream. every entity travels whole, so a lost or
/// skipped message is repaired by the next one about the same entity
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FleetEvent {
    Snapshot(Snapshot),
    Machine {
        machine: Box<MachineCard>,
    },
    Progress {
        progress: LiveProgress,
    },
    ProgressCleared {
        host: Hostname,
    },
    Deployment {
        deployment: DeploymentView,
    },
    DeploymentRemoved {
        #[cfg_attr(test, ts(type = "OrderId"))]
        order: String,
    },
    /// the session ended; the stream closes after this
    Expired,
}

/// a machine, all but its running job's progress
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct MachineCard {
    pub host: Hostname,
    pub link: LinkView,
    /// what the machine last reported, kept while it is out of reach; unset
    /// until it is first reached, since only the machine knows its platform
    pub state: Option<MachineState>,
}

/// a machine as its daemon last described it
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct MachineState {
    pub seen_at: Timestamp,
    pub platform: PlatformView,
    /// `aarch64-darwin`, `x86_64-linux`, ...
    pub system: String,
    /// the daemon's coeus version
    pub version: String,
    pub daemon: DaemonStateView,
    pub generation: Option<u32>,
    /// `/run/current-system`
    pub current: Option<String>,
    pub pending_boot: bool,
    pub reboot_required: bool,
    /// the source of the job that produced the running system, if one did
    pub deployed: Option<SourceView>,
    pub follow: Option<FollowView>,
    pub running: Option<JobCard>,
    pub next: Option<JobCard>,
    /// finished jobs, newest first
    pub recent: Vec<JobCard>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "lowercase")]
pub enum PlatformView {
    NixOs,
    Darwin,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum DaemonStateView {
    Serving,
    /// draining before exiting into a newer coeusd
    Restarting,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LinkView {
    Connected,
    Connecting,
    Unreachable {
        since: Timestamp,
        reason: UnreachView,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum UnreachView {
    /// tailscale sees it offline, asleep or away; it catches up once back
    Offline {
        last_seen: Option<Timestamp>,
    },
    /// it answered and turned coeus web away
    Refused {
        detail: String,
    },
    Failed {
        detail: String,
    },
}

/// the ref a machine catches up with on its own
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct FollowView {
    pub git_ref: String,
    /// where the ref pointed when the machine last looked
    pub latest: Option<PinnedView>,
    pub paused: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct PinnedView {
    pub rev: String,
    pub committed_at: Timestamp,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct JobCard {
    pub id: u64,
    /// 16 hex digits: an order id exceeds what a javascript number holds
    #[cfg_attr(test, ts(type = "OrderId"))]
    pub order: String,
    pub source: SourceView,
    pub action: ActionView,
    pub origin: OriginView,
    pub state: JobStateView,
    pub ordered_at: Timestamp,
    pub started_at: Option<Timestamp>,
    pub finished_at: Option<Timestamp>,
    /// when the running job entered its phase
    pub phase_started_at: Option<Timestamp>,
    pub toplevel: Option<String>,
    pub attempt: u8,
    /// how many packages the running job changes, once its closure diff is known
    pub changes: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SourceView {
    Git {
        rev: String,
        git_ref: Option<String>,
    },
    Snapshot {
        label: String,
        nar_hash: String,
    },
    Generation {
        number: u32,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OriginView {
    User {
        user: String,
        host: String,
    },
    Follow,
    Github {
        repository: String,
        workflow: String,
        run_id: u64,
        attempt: u32,
        url: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum ActionView {
    Switch,
    Boot,
    Test,
    Build,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum PhaseView {
    Preparing,
    Fetching,
    Evaluating,
    Building,
    Activating,
    Verifying,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum JobStateView {
    Queued,
    Running { phase: PhaseView },
    Finished { outcome: OutcomeView },
}

/// how a job ended. a failure's last lines are left out: the log view shows
/// the end of the log itself
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OutcomeView {
    Succeeded,
    UpToDate,
    Cancelled,
    Interrupted,
    Superseded { by: u64 },
    Failed { phase: PhaseView, message: String },
    RolledBack { phase: PhaseView, message: String },
}

/// the running job's numbers, the only entity that changes several times a
/// second
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct LiveProgress {
    pub host: Hostname,
    pub job: u64,
    pub builds: TallyView,
    pub downloads: TallyView,
    pub downloaded_bytes: u64,
    pub expected_bytes: u64,
    /// the derivations building right now, by name
    pub active: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct TallyView {
    pub done: u32,
    pub expected: u32,
    pub running: u32,
    pub failed: u32,
}

/// one order across the fleet, put together from the jobs machines report
/// for it
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct DeploymentView {
    #[cfg_attr(test, ts(type = "OrderId"))]
    pub order: String,
    pub source: SourceView,
    pub origin: OriginView,
    pub action: ActionView,
    /// the commit's subject; no machine reports it yet
    pub subject: Option<String>,
    pub ordered_at: Timestamp,
    /// when the last ordered job finished
    pub settled_at: Option<Timestamp>,
    pub waves: Vec<WaveView>,
    /// ordered machines wave by wave, then the machines that follow the ref
    pub rows: Vec<DeploymentRow>,
}

/// machines ordered together
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct WaveView {
    /// what the deploy called this wave; no machine reports it yet
    pub name: Option<String>,
    pub hosts: Vec<Hostname>,
    pub started_at: Timestamp,
    pub settled_at: Option<Timestamp>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct DeploymentRow {
    pub host: Hostname,
    /// index into the deployment's waves; unset for a machine that follows
    /// the ref instead of being ordered
    pub wave: Option<u32>,
    pub verdict: Verdict,
    pub job: Option<u64>,
    pub diff: Option<DiffSummary>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Waiting,
    Running,
    /// succeeded, up to date, or superseded by a newer order
    Done,
    /// failed, rolled back, interrupted or cancelled
    Failed,
    /// follows the ref and has yet to take the commit
    Pending,
    CatchingUp,
    CaughtUp,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct DiffSummary {
    pub changes: u32,
    /// net closure growth in bytes
    pub size_delta: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct DiffView {
    pub changes: Vec<PackageChangeView>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct PackageChangeView {
    pub name: String,
    /// empty when the package is new
    pub before: Vec<String>,
    /// empty when the package is gone
    pub after: Vec<String>,
    pub size_delta: i64,
}

/// one message of a job's log stream
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LogEvent {
    Started {
        job: Box<JobCard>,
    },
    Lines {
        lines: Vec<LogLine>,
        /// the daemon's log offset after these lines, where a reconnect resumes
        end: u64,
    },
    Ended {
        outcome: OutcomeView,
    },
    /// the session ended; the stream closes after this
    Expired,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct LogLine {
    /// when coeus recorded it; nix's own lines carry no time
    pub at: Option<Timestamp>,
    pub kind: LineKind,
    pub text: String,
    /// the derivation whose builder printed it
    pub prefix: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum LineKind {
    /// coeus's own account of the job
    Coeus,
    /// the job entered a phase; the text is its name
    Phase,
    /// nix copies or downloads something
    Fetch,
    /// nix starts building a derivation
    Build,
    /// what a builder or a program printed
    Output,
    Error,
    /// the job succeeded
    Done,
    Note,
}
