use crate::common::proto::Packet;
use serde::{Deserialize, Serialize};

/// Opaque durable handle; also the caller-provided idempotency ID. Not a timestamp or queue position.
pub type JobId = u64;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeployPhase {
    Queued,
    Checkout,
    Build,
    Switch,
    Succeeded,
    Failed,
    Cancelled,
    Interrupted,
}

impl DeployPhase {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Succeeded | Self::Failed | Self::Cancelled | Self::Interrupted
        )
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JobStatus {
    pub job_id: JobId,
    pub revision: String,
    pub commit: Option<String>,
    pub phase: DeployPhase,
    pub cancel_requested: bool,
    pub message: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StatusRequest {
    /// None lists retained jobs, in acceptance order.
    pub job_id: Option<JobId>,
    pub after: Option<JobId>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StatusResponse {
    pub jobs: Vec<JobStatus>,
    pub next_after: Option<JobId>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum LogSource {
    Lifecycle,
    Stdout,
    Stderr,
    /// stdout/stderr share one PTY; their original descriptor is unknowable.
    Terminal,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LogEntry {
    pub cursor: u64,
    pub source: LogSource,
    pub text: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LogsRequest {
    pub job_id: JobId,
    /// Exclusive cursor; 0 starts at the beginning of retained output.
    pub after: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LogsResponse {
    pub job_id: JobId,
    pub entries: Vec<LogEntry>,
    pub next_cursor: u64,
    /// True if entries after the requested cursor have already been evicted.
    pub gap: bool,
    /// No future output when true; existing pages still need draining.
    pub finished: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CancelRequest {
    pub job_id: JobId,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RequestRejected {
    pub message: String,
}

macro_rules! into_packet {
    ($($payload:ident),* $(,)?) => {$(
        impl From<$payload> for Packet {
            fn from(value: $payload) -> Self { Self::$payload(Box::new(value)) }
        }
    )*};
}
into_packet!(
    StatusRequest,
    StatusResponse,
    LogsRequest,
    LogsResponse,
    CancelRequest,
    RequestRejected
);
