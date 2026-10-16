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
        let high_water = storage.high_water();
        let mut ledger = JobLedger {
            high_water,
            records,
            storage: Some(storage),
            unhealthy: false,
        };
        use crate::common::proto::DeployPhase;
        // Never-started FIFO entries may survive behind a durable uncertainty
        // barrier. Started jobs are observed, never sent back to the executor.
        let barrier = ledger
            .records
            .iter()
            .filter(|r| {
                matches!(
                    r.status.phase,
                    DeployPhase::RequiresManualSwitch | DeployPhase::Interrupted
                ) || (!r.status.phase.is_terminal() && r.status.phase != DeployPhase::Queued)
            })
            .map(|r| r.sequence)
            .max();
        if barrier.is_some() {
            eyre::ensure!(
                ledger
                    .records
                    .iter()
                    .filter(|r| r.status.phase == DeployPhase::Queued)
                    .count()
                    <= super::QUEUE_CAPACITY,
                "recovered FIFO exceeds bound; reconcile offline"
            );
        }
        // A crash interrupts started work. Preserve only never-started queued jobs
        // behind that barrier; the worker claims Checkout durably before effects.
        let unfinished: Vec<_> = ledger
            .records
            .iter()
            .filter(|r| {
                !r.status.phase.is_terminal()
                    && r.status.phase != crate::common::proto::DeployPhase::RequiresManualSwitch
                    && !(r.status.phase == crate::common::proto::DeployPhase::Queued
                        && !r.started
                        && barrier.is_some_and(|sequence| r.sequence > sequence))
            })
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
