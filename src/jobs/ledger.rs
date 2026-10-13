use crate::common::proto::{
    DeployPhase, DeployRequest, JobId, JobStatus, LogEntry, LogSource, LogsResponse,
};
use eyre::{Result, eyre};
use std::collections::VecDeque;
use tokio_util::sync::CancellationToken;

pub(super) const MAX_JOBS: usize = 128;
pub(super) const MAX_LOGS: usize = 256;
pub(super) const LOG_PAGE: usize = 32;
pub(crate) const MAX_TEXT: usize = 1024;

pub(crate) fn bounded_text(text: &str) -> String {
    crate::utils::plain_text::bounded(text, MAX_TEXT)
}

pub(super) struct JobRecord {
    pub status: JobStatus,
    pub cancellation: CancellationToken,
    pub(super) entries: VecDeque<LogEntry>,
    pub(super) cursor: u64,
    pub request: DeployRequest,
    pub sequence: u64,
}

impl JobRecord {
    pub fn new(request: DeployRequest, sequence: u64, cancellation: CancellationToken) -> Self {
        Self {
            status: JobStatus {
                job_id: request.request_id,
                revision: request.rev.clone(),
                commit: None,
                phase: DeployPhase::Queued,
                cancel_requested: false,
                message: None,
            },
            request,
            sequence,
            cancellation,
            entries: VecDeque::new(),
            cursor: 0,
        }
    }

    pub fn append(&mut self, source: LogSource, text: &str) {
        self.cursor += 1;
        if self.entries.len() == MAX_LOGS {
            self.entries.pop_front();
        }
        self.entries.push_back(LogEntry {
            cursor: self.cursor,
            source,
            text: bounded_text(text),
        });
    }

    pub fn logs(&self, after: u64) -> Result<LogsResponse> {
        if after > self.cursor {
            return Err(eyre!("log cursor is ahead of this job"));
        }
        let gap = self
            .entries
            .front()
            .is_some_and(|entry| after < entry.cursor - 1);
        let entries: Vec<_> = self
            .entries
            .iter()
            .filter(|entry| entry.cursor > after)
            .take(LOG_PAGE)
            .cloned()
            .collect();
        let next_cursor = entries.last().map_or(after, |entry| entry.cursor);
        Ok(LogsResponse {
            job_id: self.status.job_id,
            entries,
            next_cursor,
            gap,
            finished: self.status.phase.is_terminal(),
        })
    }
}

#[derive(Default)]
pub(super) struct JobLedger {
    pub records: VecDeque<JobRecord>,
    pub storage: Option<std::sync::Arc<super::storage::JobStorage>>,
    pub unhealthy: bool,
}

impl JobLedger {
    pub fn record(&self, id: JobId) -> Result<&JobRecord> {
        self.records
            .iter()
            .find(|record| record.status.job_id == id)
            .ok_or_else(|| eyre!("unknown or expired job {id}"))
    }
    pub fn record_mut(&mut self, id: JobId) -> Result<&mut JobRecord> {
        self.records
            .iter_mut()
            .find(|record| record.status.job_id == id)
            .ok_or_else(|| eyre!("unknown or expired job {id}"))
    }
    pub fn make_room(&mut self) -> Result<()> {
        if self.records.len() < MAX_JOBS {
            return Ok(());
        }
        if self.storage.is_some() {
            eyre::bail!("durable history is full; explicit archival required before new admission");
        }
        let index = self
            .records
            .iter()
            .position(|record| record.status.phase.is_terminal())
            .ok_or_else(|| eyre!("job history is full"))?;
        self.records.remove(index);
        Ok(())
    }
}
