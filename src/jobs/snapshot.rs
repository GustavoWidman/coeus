//! Versioned on-disk schema and bounded field validation.
use super::ledger::{JobRecord, MAX_LOGS, MAX_TEXT};
use crate::common::proto::{DeployRequest, JobStatus, LogEntry};
use eyre::{Result, bail};
use serde::{Deserialize, Serialize};
use std::{collections::VecDeque, path::Path};
use tokio_util::sync::CancellationToken;
const FORMAT_VERSION: u8 = 2;

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Snapshot {
    pub(super) version: u8,
    pub(super) scope: String,
    pub(super) sequence: u64,
    pub(super) request: DeployRequest,
    pub(super) status: JobStatus,
    pub(super) entries: VecDeque<LogEntry>,
    pub(super) cursor: u64,
    pub(super) started: bool,
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
            started: record.started,
        }
    }
    pub(super) fn validate(&self, scope: &str, path: &Path) -> Result<()> {
        let id = self.status.job_id;
        if !matches!(self.version, 1 | FORMAT_VERSION)
            || self.scope != scope
            || id == 0
            || (self.version == 1 && id != self.request.request_id)
            || path
                .file_name()
                .is_none_or(|name| name != format!("{id}.job").as_str())
            || self.sequence == 0
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
                // Retain v1's original validity contract for existing history.
                // New writes/display normalize more strictly, without migrating
                // snapshots or rejecting old ESC-less/directional fragments.
                || entry.text.chars().any(|c| c.is_control() && c != '\t')
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
            started: self.started,
        }
    }
}

/// Exact pre-v6 disk shape. Do not append optional serde fields to postcard and
/// assume EOF defaults: its positional representation requires explicit decoding.
#[derive(Serialize, Deserialize)]
pub(super) struct LegacySnapshot {
    pub version: u8,
    pub scope: String,
    pub sequence: u64,
    pub request: DeployRequest,
    pub status: JobStatus,
    pub entries: VecDeque<LogEntry>,
    pub cursor: u64,
}
impl Snapshot {
    pub(super) fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.first() == Some(&1) {
            let (old, rest): (LegacySnapshot, _) = postcard::take_from_bytes(bytes)?;
            if !rest.is_empty() {
                bail!("trailing data in legacy job snapshot");
            }
            // Old snapshots did not record a started bit. Nonqueued records are
            // conservatively treated as having run; preserve real outcome/state.
            let started = old.status.phase != crate::common::proto::DeployPhase::Queued;
            Ok(Self {
                version: old.version,
                scope: old.scope,
                sequence: old.sequence,
                request: old.request,
                status: old.status,
                entries: old.entries,
                cursor: old.cursor,
                started,
            })
        } else {
            let (snapshot, rest): (Self, _) = postcard::take_from_bytes(bytes)?;
            if !rest.is_empty() {
                bail!("trailing data in job snapshot");
            }
            Ok(snapshot)
        }
    }
}
