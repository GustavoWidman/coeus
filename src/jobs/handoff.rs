//! Passive FIFO barrier: an operator process owns activation; daemon only observes receipts.
use super::JobContext;
use crate::{
    activation::{ActivationState, ReceiptStore, directory_for_ledger},
    common::proto::{DeployPhase, LogSource},
};
use eyre::{Result, bail};
use std::path::PathBuf;

impl JobContext {
    pub fn job_id(&self) -> u64 {
        self.id
    }
    pub async fn activation_directory(&self) -> Result<PathBuf> {
        let ledger = self.ledger.lock().await;
        let storage = ledger
            .storage
            .as_ref()
            .ok_or_else(|| eyre::eyre!("exact-closure deployments require durable state"))?;
        let path = directory_for_ledger(storage.directory())?;
        Ok(if path.is_absolute() {
            path
        } else {
            std::env::current_dir()?.join(path)
        })
    }
    pub async fn wait_for_switch(&self) -> Result<()> {
        let directory = self.activation_directory().await?;
        let mut reported_unreadable = false;
        loop {
            let path = directory.clone();
            let id = self.id;
            let receipt =
                tokio::task::spawn_blocking(move || crate::activation::read_receipt(&path, id))
                    .await?;
            let receipt = match receipt {
                Ok(receipt) => receipt,
                Err(_) => {
                    if !reported_unreadable {
                        self.log(LogSource::Lifecycle, "activation receipt unreadable; outcome unknown; FIFO blocked pending independent reconciliation").await;
                        reported_unreadable = true;
                    }
                    if self.shutdown.is_cancelled() {
                        bail!("daemon stopping; unreadable handoff retained, never replayed");
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                    continue;
                }
            };
            match receipt.state {
                ActivationState::Verified => {
                    // The observer must not release FIFO while the independent operator
                    // still owns the activation lease (including its final fsync/cleanup).
                    let path = directory.clone();
                    let released =
                        tokio::task::spawn_blocking(move || ReceiptStore::open(&path).is_ok())
                            .await?;
                    if released {
                        return Ok(());
                    }
                }
                ActivationState::Cancelled => {
                    bail!("retained build cancelled before activation");
                }
                ActivationState::Uncertain | ActivationState::Activating => {
                    // No automatic retries, timeout fallback, or queue unblock on uncertainty.
                }
                ActivationState::Ready => {
                    let cancel = self
                        .ledger
                        .lock()
                        .await
                        .record(self.id)?
                        .status
                        .cancel_requested;
                    if cancel {
                        let path = directory.clone();
                        let cancelled = tokio::task::spawn_blocking(move || -> Result<bool> {
                            let Ok(store) = ReceiptStore::open(&path) else { return Ok(false) };
                            let mut receipt = store.load(id)?;
                            if receipt.state != ActivationState::Ready { return Ok(false) }
                            receipt.state = ActivationState::Cancelled;
                            receipt.message = "cancelled before activation; closure retained for explicit archival".into();
                            store.save(&receipt)?;
                            Ok(true)
                        }).await??;
                        if cancelled {
                            bail!("retained build cancelled before activation");
                        }
                    }
                }
            }
            if self.shutdown.is_cancelled() {
                bail!("daemon stopping; independent switch receipt retained");
            }
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        }
    }
    pub async fn handoff(&self) -> Result<()> {
        self.phase(DeployPhase::RequiresManualSwitch).await?;
        self.log(LogSource::Lifecycle, &format!("requires manual switch: coeus switch {} --ssh TARGET --activation-directory {}; FIFO paused", self.id, self.activation_directory().await?.display())).await;
        self.wait_for_switch().await
    }
}
