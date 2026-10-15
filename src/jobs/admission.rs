//! Atomic idempotent admission independent of the submitting connection.
use super::{
    DeploymentJobs, JobContext, QueuedDeploy,
    ledger::{JobLedger, JobRecord},
};
use crate::common::proto::{DeployRequest, JobId};
use eyre::{Result, bail};
use std::sync::Arc;
use tokio::sync::{Mutex, mpsc};
use tokio_util::sync::CancellationToken;

impl DeploymentJobs {
    /// Admission owns its task: dropping a connection cannot interrupt disk commit/queue insertion.
    pub async fn accept(&self, request: DeployRequest) -> Result<JobId> {
        validate_request(&request)?;
        let ledger = self.ledger.clone();
        let queue = self.queue.clone();
        let shutdown = self.shutdown.clone();
        let terminals = self.terminals.clone();
        let feed = self.feed.clone();
        tokio::spawn(Self::admit(
            ledger, queue, shutdown, terminals, feed, request,
        ))
        .await?
    }

    async fn admit(
        ledger: Arc<Mutex<JobLedger>>,
        queue: mpsc::Sender<QueuedDeploy>,
        shutdown: CancellationToken,
        terminals: super::terminal::Terminals,
        feed: Arc<std::sync::Mutex<super::shared_terminal::SharedOutput>>,
        request: DeployRequest,
    ) -> Result<JobId> {
        let shared_ledger = ledger.clone();
        let mut ledger = ledger.lock().await;
        ledger.check_health()?;
        if let Some(record) = ledger
            .records
            .iter()
            .find(|r| r.request.request_id == request.request_id)
        {
            if record.request != request {
                bail!("request ID conflicts with previously accepted options");
            }
            return Ok(record.status.job_id);
        }
        if ledger
            .records
            .iter()
            .any(|record| record.status.phase == crate::common::proto::DeployPhase::Interrupted)
        {
            bail!("interrupted deployment requires operator reconciliation; new admission blocked");
        }
        if shutdown.is_cancelled() {
            bail!("daemon is shutting down");
        }
        if ledger
            .records
            .iter()
            .filter(|r| r.status.phase == crate::common::proto::DeployPhase::Queued)
            .count()
            >= super::QUEUE_CAPACITY
        {
            bail!("deployment queue is full");
        }
        let permit = queue
            .try_reserve()
            .map_err(|_| eyre::eyre!("deployment queue is full or closed"))?;
        // Keep record eviction and raw-history removal atomic to terminal readers.
        // All operations needing both locks use ledger -> terminal map -> output;
        // raw writers only lock their own output and never await a ledger lock.
        let mut terminals = terminals.lock().await;
        if let Some(expired) = ledger.make_room()? {
            terminals.remove(&expired);
        }
        let id = ledger
            .high_water
            .checked_add(1)
            .ok_or_else(|| eyre::eyre!("job ID space exhausted"))?;
        if let Some(storage) = &ledger.storage
            && let Err(error) = storage.reserve_id(id).await
        {
            ledger.unhealthy = true;
            for record in &mut ledger.records {
                if !record.status.phase.is_terminal() {
                    record.status.phase = crate::common::proto::DeployPhase::Interrupted;
                    record.status.message = Some(
                        "ID counter persistence failure; execution stopped; outcome uncertain"
                            .into(),
                    );
                    record.cancellation.cancel();
                }
            }
            return Err(error.wrap_err("persisting job ID high-water; admission blocked"));
        }
        ledger.high_water = id;
        let sequence = ledger
            .records
            .back()
            .map_or(Some(1), |record| record.sequence.checked_add(1))
            .ok_or_else(|| eyre::eyre!("job sequence exhausted"))?;
        let cancellation = shutdown.child_token();
        ledger.records.push_back(JobRecord::new(
            request.clone(),
            sequence,
            cancellation.clone(),
        ));
        if let Some(record) = ledger.records.back_mut() {
            record.status.job_id = id;
        }
        ledger.persist(id).await?;
        let terminal = Arc::new(std::sync::Mutex::new(
            super::terminal::TerminalOutput::default(),
        ));
        terminals.insert(id, terminal.clone());
        permit.send(QueuedDeploy {
            request,
            context: JobContext {
                id,
                feed,
                ledger: shared_ledger,
                cancellation,
                shutdown,
                terminal,
            },
        });
        Ok(id)
    }
}

pub(super) fn validate_request(request: &DeployRequest) -> Result<()> {
    if request.request_id == 0 {
        bail!("nonzero request ID required for safe acknowledgement retries");
    }
    if request.rev.is_empty()
        || request.rev.len() > 256
        || request.rev.chars().any(char::is_control)
    {
        bail!("revision must be 1..256 bytes without control characters");
    }
    Ok(())
}
