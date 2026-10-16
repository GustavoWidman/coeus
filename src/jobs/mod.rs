mod admission;
mod context;
#[cfg(test)]
mod durable_tests;
#[cfg(test)]
mod fleet_tests;
mod handoff;
#[cfg(test)]
mod handoff_tests;
mod interrupted;
#[cfg(test)]
mod interrupted_tests;
mod ledger;
mod operator_recovery;
#[cfg(test)]
mod output_tests;
mod persistence;
pub(crate) use operator_recovery::{ExistingJob, existing_job};
#[cfg(test)]
mod process_tests;
mod queries;
mod recovery;
#[cfg(test)]
mod settlement_tests;
mod shared_terminal;
mod snapshot;
mod storage;
mod switch_failure;
mod terminal;
#[cfg(test)]
mod tests;
mod worker;

use crate::common::proto::DeployRequest;
pub use context::JobContext;
use eyre::Result;
use futures_util::future::BoxFuture;
use ledger::JobLedger;
use std::sync::Arc;
use tokio::sync::{Mutex, mpsc};
use tokio_util::sync::CancellationToken;

const QUEUE_CAPACITY: usize = 16;

/// Execution boundary: tests supply fakes; production supplies Deployer.
pub trait DeploymentExecutor: Send + Sync + 'static {
    fn execute(&self, request: DeployRequest, context: JobContext) -> BoxFuture<'_, Result<()>>;
}

struct QueuedDeploy {
    request: DeployRequest,
    context: JobContext,
}

pub struct DeploymentJobs {
    queue: mpsc::Sender<QueuedDeploy>,
    ledger: Arc<Mutex<JobLedger>>,
    shutdown: CancellationToken,
    terminals: terminal::Terminals,
    feed: Arc<std::sync::Mutex<shared_terminal::SharedOutput>>,
    worker: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl DeploymentJobs {
    pub fn new(executor: Arc<dyn DeploymentExecutor>) -> Self {
        Self::start(executor, JobLedger::default())
    }

    fn start(executor: Arc<dyn DeploymentExecutor>, mut ledger: JobLedger) -> Self {
        ledger.high_water = ledger.high_water.max(
            ledger
                .records
                .iter()
                .map(|r| r.status.job_id)
                .max()
                .unwrap_or(0),
        );
        // One executing handoff plus the bounded waiting queue may survive a controlled restart.
        let (queue, receiver) = mpsc::channel(QUEUE_CAPACITY + 1);
        let shutdown = CancellationToken::new();
        for record in &mut ledger.records {
            record.cancellation = shutdown.child_token();
            if record.status.cancel_requested {
                record.cancellation.cancel();
            }
        }
        let recovered: Vec<_> = ledger
            .records
            .iter()
            .filter(|r| {
                matches!(
                    r.status.phase,
                    crate::common::proto::DeployPhase::RequiresManualSwitch
                        | crate::common::proto::DeployPhase::Interrupted
                        | crate::common::proto::DeployPhase::Queued
                )
            })
            .map(|r| (r.request.clone(), r.status.job_id, r.cancellation.clone()))
            .collect();
        let ledger = Arc::new(Mutex::new(ledger));
        let feed = Arc::new(std::sync::Mutex::new(
            shared_terminal::SharedOutput::default(),
        ));
        let mut recovered_jobs = Vec::new();
        for (request, id, cancellation) in recovered {
            let context = JobContext {
                id,
                feed: feed.clone(),
                ledger: ledger.clone(),
                cancellation,
                shutdown: shutdown.clone(),
                terminal: Arc::new(std::sync::Mutex::new(terminal::TerminalOutput::default())),
            };
            recovered_jobs.push(QueuedDeploy { request, context });
        }
        let worker = tokio::spawn(worker::run(
            receiver,
            executor,
            shutdown.clone(),
            recovered_jobs,
        ));
        Self {
            queue,
            ledger,
            shutdown,
            terminals: Arc::default(),
            feed,
            worker: Mutex::new(Some(worker)),
        }
    }

    pub async fn shutdown(&self) {
        self.shutdown.cancel();
        if let Some(worker) = self.worker.lock().await.take() {
            let _ = worker.await;
        }
    }
}

impl Drop for DeploymentJobs {
    fn drop(&mut self) {
        self.shutdown.cancel();
    }
}

pub(crate) fn bounded_error(error: &eyre::Report) -> String {
    ledger::bounded_text(&format!("{error:#}"))
}
