//! Authenticated machine-level queries. Available only after the versioned Noise handshake.
use super::{JobId, JobStatus, TerminalSize};
use crate::common::proto::Packet;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct HelloRequest {}
#[derive(Debug, Serialize, Deserialize)]
pub struct HelloResponse {
    pub protocol_version: u8,
    pub version: String,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct MachineStatusRequest {}
#[derive(Debug, Serialize, Deserialize)]
pub struct MachineStatusResponse {
    pub active: Option<JobStatus>,
    /// Most recently started job (or newest accepted job before any execution).
    /// Its real outcome is preserved; idle does not imply successful deployment.
    pub latest: Option<JobStatus>,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct SharedTerminalRequest {
    /// Daemon-global absolute byte cursor. None starts at the current live edge.
    pub after: Option<u64>,
    pub size: TerminalSize,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SharedTerminalChunk {
    pub job_id: JobId,
    pub bytes: Vec<u8>,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct SharedTerminalResponse {
    pub chunks: Vec<SharedTerminalChunk>,
    pub next_cursor: u64,
    /// Cursor preceded retained output. Resume from next_cursor, reporting lost bytes.
    pub gap: bool,
}
macro_rules! packets {
    ($($name:ident),*) => {$ (
        impl From<$name> for Packet {
            fn from(value: $name) -> Self { Self::$name(Box::new(value)) }
        }
    )*};
}
packets!(
    HelloRequest,
    HelloResponse,
    MachineStatusRequest,
    MachineStatusResponse,
    SharedTerminalRequest,
    SharedTerminalResponse
);
