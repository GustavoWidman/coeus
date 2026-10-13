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
        let result = if job.context.check_cancelled().is_err() {
            Err(eyre::eyre!("deployment cancelled before execution"))
        } else {
            // A panic must not silently kill the FIFO worker and strand all later jobs.
            let execution = executor.clone();
            let context = job.context.clone();
            match tokio::spawn(async move { execution.execute(job.request, context).await }).await {
                Ok(result) => result,
                Err(_) => Err(eyre::eyre!("deployment executor task failed")),
            }
        };
        let mut ledger = job.context.ledger.lock().await;
        if ledger.check_health().is_err() {
            continue;
        }
        if let Ok(record) = ledger.record_mut(job.context.id) {
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
