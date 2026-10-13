mod admission;
mod context;
#[cfg(test)]
mod durable_tests;
mod ledger;
#[cfg(test)]
mod output_tests;
mod persistence;
#[cfg(test)]
mod process_tests;
mod queries;
mod recovery;
mod snapshot;
mod storage;
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
    worker: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl DeploymentJobs {
    pub fn new(executor: Arc<dyn DeploymentExecutor>) -> Self {
        Self::start(executor, JobLedger::default())
    }

    fn start(executor: Arc<dyn DeploymentExecutor>, ledger: JobLedger) -> Self {
        let (queue, receiver) = mpsc::channel(QUEUE_CAPACITY);
        let ledger = Arc::new(Mutex::new(ledger));
        let shutdown = CancellationToken::new();
        let worker = tokio::spawn(worker::run(receiver, executor, shutdown.clone()));
        Self {
            queue,
            ledger,
            shutdown,
            terminals: Arc::default(),
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
