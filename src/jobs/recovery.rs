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
        let controlled = ledger
            .records
            .iter()
            .any(|r| r.status.phase == crate::common::proto::DeployPhase::RequiresManualSwitch);
        if controlled {
            eyre::ensure!(
                ledger
                    .records
                    .iter()
                    .filter(|r| r.status.phase == crate::common::proto::DeployPhase::Queued)
                    .count()
                    <= super::QUEUE_CAPACITY,
                "recovered FIFO exceeds bound; reconcile offline"
            );
            eyre::ensure!(
                !ledger
                    .records
                    .iter()
                    .any(|r| r.status.phase == crate::common::proto::DeployPhase::Interrupted),
                "interrupted work conflicts with controlled handoff; reconcile offline"
            );
            eyre::ensure!(
                ledger
                    .records
                    .iter()
                    .filter(|r| r.status.phase
                        == crate::common::proto::DeployPhase::RequiresManualSwitch)
                    .count()
                    == 1,
                "multiple manual handoffs in ledger; reconcile offline"
            );
            let barrier = ledger
                .records
                .iter()
                .find(|r| r.status.phase == crate::common::proto::DeployPhase::RequiresManualSwitch)
                .map(|r| r.sequence)
                .ok_or_else(|| eyre::eyre!("missing barrier"))?;
            eyre::ensure!(
                ledger.records.iter().all(|r| r.status.phase
                    != crate::common::proto::DeployPhase::Queued
                    || r.sequence > barrier),
                "queued job predates manual barrier; reconcile offline"
            );
        }
        // An unrelated crash still interrupts all work. Only a durable manual handoff
        // preserves never-started queued jobs; the worker claims Checkout before effects.
        let unfinished: Vec<_> = ledger
            .records
            .iter()
            .filter(|r| {
                !r.status.phase.is_terminal()
                    && r.status.phase != crate::common::proto::DeployPhase::RequiresManualSwitch
                    && !(controlled && r.status.phase == crate::common::proto::DeployPhase::Queued)
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
