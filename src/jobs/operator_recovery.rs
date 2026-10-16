//! Read-only existing-ledger acquisition for a local stopped-daemon operator.
use super::storage::JobStorage;
use crate::common::proto::{DeployRequest, JobStatus};
use eyre::{Result, ensure};
use std::{path::PathBuf, sync::Arc};

pub(crate) struct ExistingJob {
    // This lifetime is the exclusion against a running/restarting daemon.
    _lease: Arc<JobStorage>,
    pub request: DeployRequest,
    pub status: JobStatus,
    pub sequence: u64,
    pub started: bool,
    pub snapshot: Vec<u8>,
    pub high_water: u64,
}

pub(crate) fn existing_job(directory: PathBuf, scope: String, id: u64) -> Result<ExistingJob> {
    let (storage, records) = JobStorage::existing(directory, scope.clone())?;
    for name in [
        "snapshot.pending",
        "id-counter.pending",
        "ledger-format.pending",
    ] {
        ensure!(
            !storage.directory().join(name).try_exists()?,
            "ledger has pending writes; operator recovery refused"
        );
    }
    let record = records
        .iter()
        .find(|r| r.status.job_id == id)
        .ok_or_else(|| eyre::eyre!("job is not retained in this service ledger"))?;
    let snapshot = std::fs::read(storage.directory().join(format!("{id}.job")))?;
    Ok(ExistingJob {
        request: record.request.clone(),
        status: record.status.clone(),
        sequence: record.sequence,
        started: record.started,
        snapshot,
        high_water: storage.high_water(),
        _lease: storage,
    })
}
