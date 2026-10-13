//! Conservative recovery: persist uncertainty, never replay a host mutation.
use super::{DeploymentExecutor, DeploymentJobs, ledger::JobLedger, storage};
use eyre::Result;
use std::sync::Arc;

impl DeploymentJobs {
    pub async fn open(
        executor: Arc<dyn DeploymentExecutor>,
        directory: std::path::PathBuf,
        scope: String,
    ) -> Result<Self> {
        let (storage, records) = storage::JobStorage::open(directory, scope).await?;
        let mut ledger = JobLedger {
            records,
            storage: Some(storage),
            unhealthy: false,
        };
        // Even queued work is not replayed. A previous executor/child may still exist.
        let unfinished: Vec<_> = ledger
            .records
            .iter()
            .filter(|r| !r.status.phase.is_terminal())
            .map(|r| r.status.job_id)
            .collect();
        for id in unfinished {
            let record = ledger.record_mut(id)?;
            record.status.phase = crate::common::proto::DeployPhase::Interrupted;
            record.status.message =
                Some("daemon restarted; outcome uncertain; reconcile before resubmitting".into());
            record.append(
                crate::common::proto::LogSource::Lifecycle,
                "Interrupted after daemon restart; not replayed",
            );
            ledger.persist(id).await?;
        }
        Ok(Self::start(executor, ledger))
    }
}
