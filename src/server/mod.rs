mod connection;
#[cfg(test)]
mod connection_tests;
mod dispatch;
mod shutdown;
#[cfg(test)]
mod tests;

use crate::{
    common::stream::{EncryptedListener, EncryptedStream},
    config::ServerConfig,
    deploy::Deployer,
    jobs::{DeploymentExecutor, DeploymentJobs},
};
use eyre::Result;
use log::{debug, error, info};
use std::sync::Arc;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

pub struct CoeusServer {
    listener: EncryptedListener,
    jobs: DeploymentJobs,
}

impl CoeusServer {
    pub async fn new(config: ServerConfig) -> Result<Self> {
        let executor = Arc::new(Deployer::new(config.clone()).await?);
        Self::with_durable_executor(
            config.address(),
            &config.key,
            executor,
            config.state_directory.clone(),
            config.repo_url.clone(),
        )
        .await
    }

    /// Volatile execution boundary for isolated integration tests; no durable ACK claim.
    pub async fn with_executor(
        address: std::net::SocketAddr,
        key: &[u8; 32],
        executor: Arc<dyn DeploymentExecutor>,
    ) -> Result<Self> {
        let listener = EncryptedListener::bind(address, key).await?;
        Ok(Self {
            listener,
            jobs: DeploymentJobs::new(executor),
        })
    }
    pub async fn with_durable_executor(
        address: std::net::SocketAddr,
        key: &[u8; 32],
        executor: Arc<dyn DeploymentExecutor>,
        directory: std::path::PathBuf,
        scope: String,
    ) -> Result<Self> {
        let jobs = DeploymentJobs::open(executor, directory, scope).await?;
        let listener = EncryptedListener::bind(address, key).await?;
        Ok(Self { listener, jobs })
    }

    pub fn local_addr(&self) -> std::io::Result<std::net::SocketAddr> {
        self.listener.local_addr()
    }

    pub async fn run(self) -> Result<()> {
        let shutdown = CancellationToken::new();
        let signal = shutdown.clone();
        let handler = tokio::spawn(async move {
            if shutdown::signal().await.is_ok() {
                signal.cancel();
            }
        });
        let result = self.run_until_cancelled(shutdown).await;
        handler.abort();
        result
    }

    pub async fn run_until_cancelled(self, shutdown: CancellationToken) -> Result<()> {
        info!("coeus server listening on {}", self.local_addr()?);
        let server = Arc::new(self);
        let mut connections = JoinSet::new();
        loop {
            tokio::select! {
                biased;
                _ = shutdown.cancelled() => break,
                Some(_) = connections.join_next(), if !connections.is_empty() => {},
                // Tokio TCP accept is cancellation-safe. Noise belongs to the spawned
                // task, not a sibling select branch; reaping another task cannot close it.
                accepted = server.listener.accept_pending(),
                    if connections.len() < connection::MAX_CONNECTIONS => {
                    match accepted {
                        Ok(pending) => {
                            let server = server.clone();
                            connections.spawn(async move {
                                if let Err(error) = server.authenticate_and_handle(pending).await {
                                    debug!("connection ended: {error}");
                                }
                            });
                        }
                        Err(error) => error!("TCP accept failed: {error}"),
                    }
                }
            }
        }
        connections.abort_all();
        while connections.join_next().await.is_some() {}
        server.jobs.shutdown().await;
        Ok(())
    }

    pub async fn handle_connection(&self, mut stream: EncryptedStream) -> Result<()> {
        loop {
            let Some(packet) =
                tokio::time::timeout(std::time::Duration::from_secs(120), stream.recv()).await??
            else {
                return Ok(());
            };
            let response = self.dispatch(packet).await;
            tokio::time::timeout(std::time::Duration::from_secs(10), stream.send(response))
                .await??;
        }
    }
}
