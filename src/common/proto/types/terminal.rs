use super::JobId;
use crate::common::proto::Packet;
use serde::{Deserialize, Serialize};

pub const TERMINAL_PAGE_BYTES: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalSize {
    pub rows: u16,
    pub cols: u16,
}
impl Default for TerminalSize {
    fn default() -> Self {
        Self { rows: 24, cols: 80 }
    }
}
impl TerminalSize {
    pub fn validate(self) -> eyre::Result<Self> {
        eyre::ensure!(
            (1..=500).contains(&self.rows) && (1..=500).contains(&self.cols),
            "terminal size must be 1..500 rows/columns"
        );
        Ok(self)
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TerminalRequest {
    pub job_id: JobId,
    /// Absolute byte position. None explicitly skips retained history (live attach).
    pub after: Option<u64>,
    pub size: TerminalSize,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct TerminalResponse {
    pub job_id: JobId,
    pub bytes: Vec<u8>,
    pub next_cursor: u64,
    pub gap: bool,
    /// False after daemon restart: raw replay is deliberately memory-only.
    pub available: bool,
    pub finished: bool,
    /// Terminal job failure, emitted as a diagnostic only after raw pages drain.
    pub error: Option<String>,
}
impl From<TerminalRequest> for Packet {
    fn from(value: TerminalRequest) -> Self {
        Self::TerminalRequest(Box::new(value))
    }
}
impl From<TerminalResponse> for Packet {
    fn from(value: TerminalResponse) -> Self {
        Self::TerminalResponse(Box::new(value))
    }
}
