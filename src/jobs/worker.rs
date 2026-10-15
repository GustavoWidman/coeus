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
        let mut safe_terminal = true;
        if let Err(error) = &result {
            if error.is::<super::switch_failure::RevocationFailure>() {
                stop_after_persistence_failure(&job.context, format!("{error:#}")).await;
                return;
            }
            let cause = format!("{error:#}");
            let (phase, durable, cancel) = {
                let ledger = job.context.ledger.lock().await;
                let Ok(record) = ledger.record(job.context.id) else {
                    return;
                };
                (
                    record.status.phase,
                    ledger.storage.is_some(),
                    record.status.cancel_requested,
                )
            };
            let required = matches!(
                phase,
                DeployPhase::Switch | DeployPhase::RequiresManualSwitch
            );
            if durable || required {
                job.context
                    .log(LogSource::Lifecycle, &format!("activation failed: {cause}"))
                    .await;
                // No phase-based exemption: a Ready receipt may already exist while
                // the ledger still says Build, or a build-only revocation may fail.
                let revoke = phase != DeployPhase::RequiresManualSwitch || cancel;
                let settled =
                    super::switch_failure::settle(&job.context, cause.clone(), revoke, required)
                        .await;
                use super::switch_failure::Settlement;
                match settled {
                    Ok(Settlement::Absent | Settlement::Cancelled) => {}
                    Ok(Settlement::Verified) => result = Ok(()),
                    Err(error) if error.is::<super::switch_failure::RevocationFailure>() => {
                        stop_after_persistence_failure(&job.context, format!("{cause}; {error:#}"))
                            .await;
                        return;
                    }
                    pending => {
                        if let Err(error) = pending {
                            job.context
                                .log(
                                    LogSource::Lifecycle,
                                    &format!(
                                        "activation receipt could not be settled safely: {error:#}"
                                    ),
                                )
                                .await;
                        }
                        let mut ledger = job.context.ledger.lock().await;
                        if let Ok(record) = ledger.record_mut(job.context.id) {
                            record.status.phase = DeployPhase::RequiresManualSwitch;
                            record.status.message = Some(bounded_text(&format!(
                                "activation outcome uncertain; reconcile receipt, FIFO paused: {cause}"
                            )));
                        }
                        if ledger.persist(job.context.id).await.is_err() {
                            return;
                        }
                        drop(ledger);
                        use eyre::WrapErr;
                        result = job.context.wait_for_switch().await.wrap_err(cause);
                        if let Err(error) = &result {
                            if error.is::<super::switch_failure::RevocationFailure>() {
                                stop_after_persistence_failure(&job.context, format!("{error:#}"))
                                    .await;
                                return;
                            }
                            // Only a fresh leased, durable terminal observation can
                            // release the barrier after an observer error/shutdown.
                            match super::switch_failure::settle(
                                &job.context,
                                "handoff observer stopped".into(),
                                false,
                                true,
                            )
                            .await
                            {
                                Ok(Settlement::Cancelled) => {}
                                Ok(Settlement::Verified) => result = Ok(()),
                                Err(error)
                                    if error.is::<super::switch_failure::RevocationFailure>() =>
                                {
                                    stop_after_persistence_failure(
                                        &job.context,
                                        format!("{error:#}"),
                                    )
                                    .await;
                                    return;
                                }
                                _ => safe_terminal = false,
                            }
                        }
                    }
                }
            }
        }
        let mut ledger = job.context.ledger.lock().await;
        if ledger.check_health().is_err() {
            return;
        }
        if let Ok(record) = ledger.record_mut(job.context.id) {
            if record.status.phase == DeployPhase::RequiresManualSwitch
                && result.is_err()
                && !safe_terminal
            {
                // No queued operation can pass an unresolved/corrupt handoff. On restart
                // the passive observer resumes; activation is never replayed.
                record.status.message = Some(bounded_text(&format!(
                    "independent handoff unresolved; inspect/reconcile receipt; FIFO blocked: {:#}",
                    result
                        .as_ref()
                        .err()
                        .map(|e| format!("{e:#}"))
                        .unwrap_or_default()
                )));
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

async fn stop_after_persistence_failure(context: &super::JobContext, cause: String) {
    let mut ledger = context.ledger.lock().await;
    if let Ok(record) = ledger.record_mut(context.id) {
        record.status.phase = DeployPhase::Interrupted;
        record.status.message = Some(bounded_text(&cause));
        record.append(LogSource::Lifecycle, &cause);
    }
    // Persist uncertainty, never a safe terminal outcome. If this fails too,
    // ledger recovery still treats the old Build/manual state conservatively.
    let _ = ledger.persist(context.id).await;
    ledger.unhealthy = true;
}
