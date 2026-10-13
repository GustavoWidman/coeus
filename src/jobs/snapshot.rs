//! Versioned on-disk schema and bounded field validation.
use super::ledger::{JobRecord, MAX_JOBS, MAX_LOGS, MAX_TEXT};
use crate::common::proto::{DeployRequest, JobStatus, LogEntry};
use eyre::{Result, bail};
use serde::{Deserialize, Serialize};
use std::{collections::VecDeque, path::Path};
use tokio_util::sync::CancellationToken;
const FORMAT_VERSION: u8 = 1;

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Snapshot {
    pub(super) version: u8,
    pub(super) scope: String,
    pub(super) sequence: u64,
    pub(super) request: DeployRequest,
    pub(super) status: JobStatus,
    pub(super) entries: VecDeque<LogEntry>,
    pub(super) cursor: u64,
}

impl Snapshot {
    pub(super) fn from_record(record: &JobRecord, scope: String) -> Self {
        Self {
            version: FORMAT_VERSION,
            scope,
            sequence: record.sequence,
            request: record.request.clone(),
            status: record.status.clone(),
            entries: record.entries.clone(),
            cursor: record.cursor,
        }
    }
    pub(super) fn validate(&self, scope: &str, path: &Path) -> Result<()> {
        let id = self.status.job_id;
        if self.version != FORMAT_VERSION
            || self.scope != scope
            || id == 0
            || id != self.request.request_id
            || path
                .file_name()
                .is_none_or(|name| name != format!("{id}.job").as_str())
            || self.sequence == 0
            || self.sequence > MAX_JOBS as u64
            || self.status.revision != self.request.rev
        {
            bail!("incompatible or inconsistent job snapshot");
        }
        super::admission::validate_request(&self.request)?;
        if self.entries.len() > MAX_LOGS
            || self
                .status
                .message
                .as_ref()
                .is_some_and(|text| text.len() > MAX_TEXT)
            || self.status.commit.as_ref().is_some_and(|commit| {
                !matches!(commit.len(), 40 | 64) || !commit.bytes().all(|b| b.is_ascii_hexdigit())
            })
        {
            bail!("job snapshot fields exceed bounds");
        }
        if self.cursor > u64::MAX - 4096 {
            bail!("job log cursor exhausted");
        }
        let mut previous = self.cursor.saturating_sub(self.entries.len() as u64);
        for entry in &self.entries {
            if entry.cursor != previous + 1
                || entry.text.len() > MAX_TEXT
                || super::ledger::bounded_text(&entry.text) != entry.text
            {
                bail!("invalid retained log cursor or size");
            }
            previous = entry.cursor;
        }
        if previous != self.cursor {
            bail!("invalid job log cursor");
        }
        Ok(())
    }

    pub(super) fn into_record(self) -> JobRecord {
        JobRecord {
            status: self.status,
            request: self.request,
            sequence: self.sequence,
            cancellation: CancellationToken::new(),
            entries: self.entries,
            cursor: self.cursor,
        }
    }
}
