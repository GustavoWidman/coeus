use super::{DeploymentExecutor, QueuedDeploy, ledger::bounded_text};
use crate::common::proto::{DeployPhase, LogSource};
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

pub(super) async fn run(
    mut queue: mpsc::Receiver<QueuedDeploy>,
    executor: Arc<dyn DeploymentExecutor>,
    shutdown: CancellationToken,
) {
    loop {
        let next = tokio::select! {
            biased;
            _ = shutdown.cancelled() => { queue.close(); queue.recv().await }
            next = queue.recv() => next,
        };
        let Some(job) = next else {
            break;
        };
        // A recovered manual handoff is a passive receipt observer even when
        // cancellation was persisted. In-flight cancellation is not rollback:
        // only Ready -> Cancelled or verified reconciliation can release FIFO.
        let manual = job
            .context
            .ledger
            .lock()
            .await
            .record(job.context.id)
            .is_ok_and(|r| r.status.phase == DeployPhase::RequiresManualSwitch);
        let mut result = if !manual && job.context.check_cancelled().is_err() {
            Err(eyre::eyre!("deployment cancelled before execution"))
        } else {
            // A panic must not silently kill the FIFO worker and strand all later jobs.
            let execution = executor.clone();
            let context = job.context.clone();
            match tokio::spawn(async move {
                if manual {
                    context.wait_for_switch().await
                } else {
                    context.starting().await?;
                    execution.execute(job.request, context).await
                }
            })
            .await
            {
                Ok(result) => result,
                Err(_) => Err(eyre::eyre!("deployment executor task failed")),
            }
        };
        if result.is_err() {
            let phase = job
                .context
                .ledger
                .lock()
                .await
                .record(job.context.id)
                .map(|r| r.status.phase);
            let receipt_state = if let Ok(directory) = job.context.activation_directory().await {
                let id = job.context.id;
                tokio::task::spawn_blocking(move || {
                    crate::activation::read_receipt(&directory, id).map(|r| r.state)
                })
                .await
                .ok()
                .and_then(Result::ok)
            } else {
                None
            };
            if matches!(phase, Ok(DeployPhase::Switch))
                && receipt_state == Some(crate::activation::ActivationState::Verified)
            {
                result = Ok(());
            }
            if matches!(phase, Ok(DeployPhase::Switch))
                && !matches!(
                    receipt_state,
                    Some(
                        crate::activation::ActivationState::Verified
                            | crate::activation::ActivationState::Cancelled
                    )
                )
            {
                let mut ledger = job.context.ledger.lock().await;
                if let Ok(record) = ledger.record_mut(job.context.id) {
                    record.status.phase = DeployPhase::RequiresManualSwitch;
                    record.status.message =
                        Some("activation outcome uncertain; reconcile receipt, FIFO paused".into());
                }
                if ledger.persist(job.context.id).await.is_ok() {
                    drop(ledger);
                    result = job.context.wait_for_switch().await;
                }
            }
        }
        let safe_cancel = if let Ok(directory) = job.context.activation_directory().await {
            crate::activation::read_receipt(&directory, job.context.id)
                .is_ok_and(|r| r.state == crate::activation::ActivationState::Cancelled)
        } else {
            false
        };
        let mut ledger = job.context.ledger.lock().await;
        if ledger.check_health().is_err() {
            continue;
        }
        if let Ok(record) = ledger.record_mut(job.context.id) {
            if record.status.phase == DeployPhase::RequiresManualSwitch
                && result.is_err()
                && !safe_cancel
            {
                // No queued operation can pass an unresolved/corrupt handoff. On restart
                // the passive observer resumes; activation is never replayed.
                record.status.message = Some(
                    "independent handoff unresolved; inspect/reconcile receipt; FIFO blocked"
                        .into(),
                );
                if let Err(error) = ledger.persist(job.context.id).await {
                    log::error!("handoff persistence failed: {error:#}");
                }
                return;
            }

            record.status.phase = match &result {
                Ok(()) => DeployPhase::Succeeded,
                Err(_) if job.context.cancellation.is_cancelled() => DeployPhase::Cancelled,
                Err(_) => DeployPhase::Failed,
            };
            record.status.message = result
                .err()
                .map(|error| bounded_text(&format!("{error:#}")));
            let outcome = match &record.status.message {
                Some(message) => format!("{:?} — {message}", record.status.phase),
                None => format!("{:?}", record.status.phase),
            };
            // Attachments need the same failure cause as status, even when the
            // command never started and therefore produced no stdout/stderr.
            // append applies the existing chunk/count/control-character bounds.
            record.append(LogSource::Lifecycle, &outcome);
            if let Err(error) = ledger.persist(job.context.id).await {
                log::error!("job completion persistence failed: {error:#}");
            }
        }
    }
}
