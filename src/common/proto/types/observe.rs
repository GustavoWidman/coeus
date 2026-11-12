use serde::{Deserialize, Serialize};

use crate::common::{
    model::{AttachFrame, JobId, JobSelector, JobSummary, MachineView, Outcome},
    proto::Packet,
};

/// ask for the machine's view; with `follow`, keep receiving it as it changes
#[derive(Debug, Serialize, Deserialize, Packet)]
pub struct WatchRequest {
    pub follow: bool,
}

#[derive(Debug, Serialize, Deserialize, Packet)]
pub struct MachineUpdate {
    pub view: MachineView,
}

/// stream a job's log from a byte offset, live until the job finishes
#[derive(Debug, Serialize, Deserialize, Packet)]
pub struct AttachRequest {
    pub job: JobSelector,
    pub from: u64,
}

/// the job the attach resolved to, sent once before any batch
#[derive(Debug, Serialize, Deserialize, Packet)]
pub struct AttachStarted {
    pub job: JobSummary,
}

#[derive(Debug, Serialize, Deserialize, Packet)]
pub struct AttachBatch {
    pub frames: Vec<AttachFrame>,
    /// the log offset after the last frame, where a reconnect resumes
    pub end: u64,
}

#[derive(Debug, Serialize, Deserialize, Packet)]
pub struct AttachEnded {
    pub job: JobId,
    pub outcome: Outcome,
}

#[derive(Debug, Serialize, Deserialize, Packet)]
pub struct HistoryRequest {
    pub limit: u32,
}

#[derive(Debug, Serialize, Deserialize, Packet)]
pub struct HistoryResponse {
    /// newest first
    pub jobs: Vec<JobSummary>,
}
