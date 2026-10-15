//! Passive FIFO barrier: an operator process owns activation; daemon only observes receipts.
use super::JobContext;
use crate::{
    activation::directory_for_ledger,
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
        let mut reported_unreadable = false;
        let mut cancellation_seen = None;
        loop {
            let cancel = self
                .ledger
                .lock()
                .await
                .record(self.id)?
                .status
                .cancel_requested;
            match super::switch_failure::settle(
                self,
                "operator requested cancellation before activation".into(),
                cancel,
                true,
            )
            .await
            {
                Ok(super::switch_failure::Settlement::Verified) => return Ok(()),
                Ok(super::switch_failure::Settlement::Cancelled) => {
                    bail!("retained build cancelled before activation");
                }
                Ok(_) if cancel => {
                    bail!(
                        "cancellation requested after activation intent; outcome uncertain; no replay"
                    );
                }
                Ok(_) => {} // Ready/Activating/Uncertain: never replay.
                Err(error) if error.is::<super::switch_failure::RevocationFailure>() => {
                    // Do not retry a failed persistence operation or trust its
                    // readable post-rename state. Worker stops admission/FIFO.
                    return Err(error);
                }
                Err(error) if cancel => {
                    let since = cancellation_seen.get_or_insert_with(tokio::time::Instant::now);
                    if since.elapsed() >= std::time::Duration::from_secs(1) {
                        return Err(error.wrap_err(
                            "cancellation requested; activation ownership/outcome uncertain",
                        ));
                    }
                }
                Err(_) => {
                    if !reported_unreadable {
                        self.log(LogSource::Lifecycle, "activation receipt unreadable or leased; outcome unknown; FIFO blocked pending independent reconciliation").await;
                        reported_unreadable = true;
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
