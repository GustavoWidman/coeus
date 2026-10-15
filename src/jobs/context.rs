use super::ledger::JobLedger;
use crate::common::proto::{DeployPhase, JobId, LogSource};
use eyre::{Result, bail};
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
pub struct JobContext {
    pub(super) id: JobId,
    pub(super) feed: Arc<std::sync::Mutex<super::shared_terminal::SharedOutput>>,
    pub(super) ledger: Arc<Mutex<JobLedger>>,
    pub(crate) terminal: Arc<std::sync::Mutex<super::terminal::TerminalOutput>>,
    pub(super) cancellation: CancellationToken,
    pub(super) shutdown: CancellationToken,
}

impl JobContext {
    /// Claim execution durably before invoking any injected executor; no extra log event.
    pub(super) async fn starting(&self) -> Result<()> {
        let context = self.clone();
        tokio::spawn(async move {
            let mut ledger = context.ledger.lock().await;
            context.check_cancelled()?;
            let record = ledger.record_mut(context.id)?;
            record.status.phase = DeployPhase::Checkout;
            record.started = true;
            ledger.persist(context.id).await
        })
        .await?
    }

    pub(crate) fn terminal_bytes(&self, bytes: &[u8]) -> Result<()> {
        self.feed
            .lock()
            .map_err(|_| eyre::eyre!("shared terminal state poisoned"))?
            .push(self.id, bytes)?;
        self.terminal
            .lock()
            .map_err(|_| eyre::eyre!("terminal state poisoned"))?
            .push(bytes)
    }
    pub(crate) fn cancellation_token(&self) -> CancellationToken {
        self.cancellation.clone()
    }
    pub fn check_cancelled(&self) -> Result<()> {
        if self.cancellation.is_cancelled() {
            bail!("deployment cancellation requested");
        }
        Ok(())
    }
    pub async fn cancelled(&self) {
        self.cancellation.cancelled().await;
    }
    pub async fn phase(&self, phase: DeployPhase) -> Result<()> {
        let context = self.clone();
        tokio::spawn(async move { context.persist_phase(phase).await }).await?
    }
    async fn persist_phase(&self, phase: DeployPhase) -> Result<()> {
        if phase.is_terminal() {
            bail!("terminal phases belong to the worker, not the executor");
        }
        let mut ledger = self.ledger.lock().await;
        self.check_cancelled()?;
        let record = ledger.record_mut(self.id)?;
        if record.status.phase.is_terminal() {
            bail!("deployment is already terminal");
        }
        record.status.phase = phase;
        record.append(LogSource::Lifecycle, &format!("{phase:?}"));
        ledger.persist(self.id).await
    }
    pub async fn checked_out(&self, commit: String) -> Result<()> {
        let context = self.clone();
        tokio::spawn(async move { context.persist_commit(commit).await }).await?
    }
    async fn persist_commit(&self, commit: String) -> Result<()> {
        if !matches!(commit.len(), 40 | 64) || !commit.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            bail!("invalid resolved Git commit");
        }
        let mut ledger = self.ledger.lock().await;
        let record = ledger.record_mut(self.id)?;
        if record.status.phase.is_terminal() {
            bail!("deployment is already terminal");
        }
        record.append(LogSource::Lifecycle, &format!("checked out {commit}"));
        record.status.commit = Some(commit);
        ledger.persist(self.id).await
    }
    pub async fn log(&self, source: LogSource, text: &str) {
        let context = self.clone();
        let text = super::ledger::bounded_text(text);
        // The owned task retains the ledger lock even if output draining is cancelled.
        // Otherwise an older I/O snapshot could overwrite a newer terminal record.
        if let Err(error) =
            tokio::spawn(async move { context.persist_log(source, &text).await }).await
        {
            log::error!("job log task failed: {error}");
        }
    }
    async fn persist_log(&self, source: LogSource, text: &str) {
        let mut ledger = self.ledger.lock().await;
        if let Ok(record) = ledger.record_mut(self.id)
            && !record.status.phase.is_terminal()
        {
            record.append(source, text);
            if let Err(error) = ledger.persist(self.id).await {
                log::error!("job logging failed: {error:#}");
            }
        }
    }
}
