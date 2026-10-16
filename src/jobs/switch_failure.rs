//! One leased durability boundary for every receipt observer/revocation.
use super::JobContext;
use crate::activation::{ActivationState, ReceiptStore};
use eyre::Result;

#[derive(Debug)]
pub(super) struct RevocationFailure(pub String);
impl std::fmt::Display for RevocationFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for RevocationFailure {}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Settlement {
    Absent,
    Pending,
    Cancelled,
    Verified,
    OperatorResolved,
}

pub(super) async fn settle(
    context: &JobContext,
    cause: String,
    revoke_ready: bool,
    receipt_required: bool,
) -> Result<Settlement> {
    let directory = context.activation_directory().await?;
    let id = context.job_id();
    tokio::task::spawn_blocking(move || {
        observe(directory, id, cause, revoke_ready, receipt_required)
            .map(|(outcome, _lease)| outcome)
    })
    .await?
}

/// Keep the exclusive activation lease alive through the caller's ledger commit.
/// A terminal observation must not be reopened between receipt and FIFO settlement.
pub(super) fn observe(
    directory: std::path::PathBuf,
    id: u64,
    cause: String,
    revoke_ready: bool,
    receipt_required: bool,
) -> Result<(Settlement, ReceiptStore)> {
    // Even absence and terminal observations require the exclusive lease.
    let store = ReceiptStore::open(&directory)?;
    let mut receipt = match store.load(id) {
        Ok(receipt) => receipt,
        Err(error) => {
            let absent = std::fs::symlink_metadata(directory.join(format!("{id}.toml")))
                .is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound);
            let no_pending = std::fs::symlink_metadata(directory.join("receipt.pending"))
                .is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound);
            if !receipt_required && absent && no_pending {
                return Ok((Settlement::Absent, store));
            }
            return Err(error);
        }
    };
    if receipt.state == ActivationState::Ready && revoke_ready {
        receipt.state = ActivationState::Cancelled;
        receipt.message = super::ledger::bounded_text(&format!(
            "failed before activation intent; activation revoked: {cause}"
        ));
    }
    let outcome = match receipt.state {
        ActivationState::Cancelled => Settlement::Cancelled,
        ActivationState::Verified => Settlement::Verified,
        ActivationState::OperatorResolved => Settlement::OperatorResolved,
        _ => return Ok((Settlement::Pending, store)),
    };
    // A preceding writer may have renamed without directory fsync. Re-save
    // under this lease before trusting this terminal observation as durable.
    store.save(&receipt).map_err(|error| {
        RevocationFailure(format!(
            "activation settlement/revocation persistence failed: {error:#}"
        ))
    })?;
    Ok((outcome, store))
}

impl JobContext {
    pub(crate) async fn revoke_build_only(&self) -> Result<()> {
        eyre::ensure!(
            settle(self, "build-only; activation forbidden".into(), true, true).await?
                == Settlement::Cancelled,
            "build-only receipt could not be durably revoked; reconcile without replay"
        );
        Ok(())
    }
}
