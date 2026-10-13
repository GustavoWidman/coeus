//! Fail closed on ambiguous I/O: stop executors, preserve inspectable uncertainty.
use super::ledger::JobLedger;
use crate::common::proto::{DeployPhase, JobId};
use eyre::Result;

impl JobLedger {
    pub fn check_health(&self) -> Result<()> {
        if self.unhealthy {
            eyre::bail!(
                "ledger persistence failed; execution stopped; restart and reconcile status"
            );
        }
        Ok(())
    }
    pub async fn persist(&mut self, id: JobId) -> Result<()> {
        self.check_health()?;
        if let Some(storage) = &self.storage
            && let Err(error) = storage.save(self.record(id)?).await
        {
            self.unhealthy = true;
            for record in &mut self.records {
                if record.status.job_id == id || !record.status.phase.is_terminal() {
                    record.status.phase = DeployPhase::Interrupted;
                    record.status.message =
                        Some("persistence failure; deployment outcome uncertain".into());
                }
                record.cancellation.cancel();
            }
            return Err(
                error.wrap_err("job persistence failed; admission/outcome may be uncertain")
            );
        }
        Ok(())
    }
}
