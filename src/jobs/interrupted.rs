//! Interrupted is a durable safety barrier, not a dead FIFO worker.
use super::{
    JobContext,
    ledger::bounded_text,
    switch_failure::{Settlement, observe},
};
use crate::common::proto::{DeployPhase, LogSource};
use std::time::Duration;

impl JobContext {
    /// Observe only; never execute the interrupted job or infer cleanup from exit
    /// status, cancellation, a process listing, or an absent/unreadable receipt.
    pub(super) async fn observe_interrupted(&self) -> bool {
        let Ok(directory) = self.activation_directory().await else {
            return false;
        };
        loop {
            if self.shutdown.is_cancelled() {
                return false;
            }
            // Admission/cancellation and the terminal receipt-to-ledger commit share
            // this mutex. The receipt's exclusive lease survives ledger persistence.
            let mut ledger = self.ledger.lock().await;
            if ledger.check_health().is_err() {
                return false;
            }
            let Ok(record) = ledger.record(self.id) else {
                return false;
            };
            if record.status.phase != DeployPhase::Interrupted {
                return true;
            }
            let id = self.id;
            let directory = directory.clone();
            let observed = tokio::task::spawn_blocking(move || {
                observe(
                    directory,
                    id,
                    "interrupted activation observation".into(),
                    false,
                    true,
                )
            })
            .await;
            match observed {
                Ok(Ok((
                    outcome @ (Settlement::Verified
                    | Settlement::Cancelled
                    | Settlement::OperatorResolved),
                    lease,
                ))) => {
                    if outcome == Settlement::OperatorResolved {
                        let Ok(record) = ledger.record(id) else {
                            return false;
                        };
                        let Some(storage) = &ledger.storage else {
                            return false;
                        };
                        let request = record.request.clone();
                        let sequence = record.sequence;
                        let started = record.started;
                        let commit = record.status.commit.clone();
                        let high_water = ledger.high_water;
                        let ledger_directory = storage.directory().to_path_buf();
                        let retained_lease = lease.clone();
                        let valid = tokio::task::spawn_blocking(move || {
                            retained_lease
                                .load(id)
                                .ok()
                                .and_then(|receipt| receipt.operator_resolution)
                                .is_some_and(|audit| {
                                    audit.binding.job_id == id
                                        && high_water >= audit.binding.high_water
                                        && audit.binding.request_nonce == request.request_id
                                        && audit.binding.revision == request.rev
                                        && audit.binding.dry_run == request.dry_run
                                        && audit.binding.clean_substituters
                                            == request.clean_substituters
                                        && audit.binding.sequence == sequence
                                        && audit.binding.started == started
                                        && audit.binding.commit == commit
                                        && std::fs::canonicalize(ledger_directory).ok().as_ref()
                                            == Some(&audit.binding.ledger)
                                        && std::fs::canonicalize(retained_lease.directory())
                                            .ok()
                                            .as_ref()
                                            == Some(&audit.binding.activation_directory)
                                })
                        })
                        .await
                        .unwrap_or(false);
                        if !valid {
                            // A self-consistent attestation for another ledger job
                            // is not a terminal observation for this one.
                            drop(lease);
                            drop(ledger);
                            tokio::select! {
                                _ = self.shutdown.cancelled() => return false,
                                _ = tokio::time::sleep(Duration::from_millis(250)) => {}
                            }
                            continue;
                        }
                    }
                    let (phase, message) = match outcome {
                        Settlement::Verified => (
                            DeployPhase::Succeeded,
                            "activation verified by durable receipt; prior cancellation did not roll back changes",
                        ),
                        Settlement::OperatorResolved => (
                            DeployPhase::Failed,
                            "Partial activation resolved by operator attestation. Not verified success or rollback.",
                        ),
                        _ => (
                            DeployPhase::Cancelled,
                            "activation durably revoked before intent; execution stopped",
                        ),
                    };
                    if let Ok(record) = ledger.record_mut(id) {
                        record.status.phase = phase;
                        record.status.message = Some(bounded_text(message));
                        record.append(LogSource::Lifecycle, message);
                    }
                    return ledger.persist(id).await.is_ok();
                }
                Ok(Err(error)) if error.is::<super::switch_failure::RevocationFailure>() => {
                    // Do not retry a failed persistence operation within this run.
                    // A restart must obtain a fresh durable leased observation.
                    if let Ok(record) = ledger.record_mut(id) {
                        record.status.message = Some(bounded_text(&format!(
                            "recovery persistence failed; activation uncertain: {error:#}"
                        )));
                    }
                    let _ = ledger.persist(id).await;
                    ledger.unhealthy = true;
                    return false;
                }
                Ok(Ok((_, lease))) => drop(lease), // Never retain a pending lease while sleeping.
                _ => {}                            // Busy lease, missing/corrupt receipt: block.
            }
            drop(ledger);
            tokio::select! {
                _ = self.shutdown.cancelled() => return false,
                _ = tokio::time::sleep(Duration::from_millis(250)) => {}
            }
        }
    }
}
