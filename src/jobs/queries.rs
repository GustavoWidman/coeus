//! Bounded status/log reads and durable cancellation intent.
use super::{DeploymentJobs, ledger::JobLedger};
use crate::common::proto::{JobId, JobStatus, LogsResponse, StatusResponse};
use eyre::{Result, bail};
use std::sync::Arc;
use tokio::sync::Mutex;

impl DeploymentJobs {
    pub async fn status(&self, id: Option<JobId>) -> Result<Vec<JobStatus>> {
        let ledger = self.ledger.lock().await;
        match id {
            Some(id) => Ok(vec![ledger.record(id)?.status.clone()]),
            None => Ok(ledger
                .records
                .iter()
                .map(|record| record.status.clone())
                .collect()),
        }
    }
    pub async fn status_page(
        &self,
        id: Option<JobId>,
        after: Option<JobId>,
    ) -> Result<StatusResponse> {
        let ledger = self.ledger.lock().await;
        if let Some(id) = id {
            if after.is_some() {
                bail!("single-job status cannot have a page cursor");
            }
            return Ok(StatusResponse {
                jobs: vec![ledger.record(id)?.status.clone()],
                next_after: None,
            });
        }
        let start = match after {
            None => 0,
            Some(id) => {
                ledger
                    .records
                    .iter()
                    .position(|record| record.status.job_id == id)
                    .ok_or_else(|| eyre::eyre!("status cursor expired; restart listing"))?
                    + 1
            }
        };
        let jobs: Vec<_> = ledger
            .records
            .iter()
            .skip(start)
            .take(32)
            .map(|record| record.status.clone())
            .collect();
        let next_after = if start + jobs.len() < ledger.records.len() {
            jobs.last().map(|job| job.job_id)
        } else {
            None
        };
        Ok(StatusResponse { jobs, next_after })
    }
    pub async fn logs(&self, id: JobId, after: u64) -> Result<LogsResponse> {
        self.ledger.lock().await.record(id)?.logs(after)
    }
    pub async fn cancel(&self, id: JobId) -> Result<JobStatus> {
        let ledger = self.ledger.clone();
        tokio::spawn(async move { Self::persist_cancellation(ledger, id).await }).await?
    }
    async fn persist_cancellation(ledger: Arc<Mutex<JobLedger>>, id: JobId) -> Result<JobStatus> {
        let mut ledger = ledger.lock().await;
        let record = ledger.record_mut(id)?;
        if record.status.phase.is_terminal() {
            return Ok(record.status.clone());
        }
        if !record.status.phase.is_terminal() {
            record.status.cancel_requested = true;
        }
        ledger.persist(id).await?;
        let record = ledger.record(id)?;
        if record.status.cancel_requested {
            record.cancellation.cancel();
        }
        Ok(record.status.clone())
    }
}
