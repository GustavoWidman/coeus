mod shutdown;

use std::{sync::Arc, time::Duration};

use colored::Colorize;
use eyre::{Context, Result};
use log::{debug, error, info};
use tokio::{task::JoinSet, time::timeout};
use tokio_util::sync::CancellationToken;

use crate::{
    common::{
        proto::{Error, Packet},
        stream::{EncryptedListener, EncryptedStream, PendingConnection},
    },
    config::ServerConfig,
};

pub const HANDSHAKE_DEADLINE: Duration = Duration::from_secs(10);

pub struct CoeusServer {
    listener: EncryptedListener,
    config: ServerConfig,
}

impl CoeusServer {
    pub async fn new(config: ServerConfig) -> Result<Self> {
        let listener = EncryptedListener::bind(config.address(), &config.key).await?;
        Ok(Self { listener, config })
    }

    pub async fn run(self) -> Result<()> {
        let mut signals = shutdown::Signals::register()?;
        let shutdown = CancellationToken::new();

        let server = self.serve(shutdown.clone());
        tokio::pin!(server);

        tokio::select! {
            result = &mut server => result, // server crash
            signal_result = signals.wait() => {
                // graceful shutdown
                shutdown.cancel();
                let server_result = server.await;
                signal_result.and(server_result)
            },
        }
    }

    async fn serve(self, shutdown: CancellationToken) -> Result<()> {
        info!(
            "coeus server listening on {}",
            self.config.address().to_string().magenta()
        );

        let server = Arc::new(self);
        let mut connections = JoinSet::new();

        loop {
            tokio::select! {
                biased;
                _ = shutdown.cancelled() => break,
                Some(_) = connections.join_next(), if !connections.is_empty() => {},
                // Tokio TCP accept is cancellation-safe. Noise belongs to the spawned
                // task, not a sibling select branch; reaping another task cannot close it.
                accepted = server.listener.listen() => {
                    // if connections.len() < connection::MAX_CONNECTIONS => {
                    match accepted {
                        Ok(pending) => {
                            let server = server.clone();
                            connections.spawn(async move {
                                if let Err(error) = server.handle_connection(pending).await {
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
        // server.jobs.shutdown().await;
        Ok(())
    }

    pub async fn handle_connection(&self, pending: PendingConnection) -> Result<()> {
        let mut stream = timeout(HANDSHAKE_DEADLINE, pending.authenticate())
            .await
            .wrap_err("Noise handshake deadline reached")??;

        loop {
            let packet = stream.recv().await.inspect_err(|e| {
                error!(
                    "error receiving packet from {}:\n{}",
                    stream
                        .peer_addr()
                        .map(|addr| addr.to_string().magenta().to_string())
                        .unwrap_or_else(|_| "unknown".to_string()),
                    e
                );
            })?;

            match packet {
                Some(packet) => {
                    // TODO: handle packets asynchronously
                    self.handle_packet(&mut stream, packet)
                        .await
                        .inspect_err(|e| {
                            error!(
                                "error handling packet from {}:\n{}",
                                stream
                                    .peer_addr()
                                    .map(|addr| addr.to_string().magenta().to_string())
                                    .unwrap_or_else(|_| "unknown".to_string()),
                                e
                            );
                        })?;
                }
                None => {
                    debug!(
                        "connection from {} closed",
                        stream
                            .peer_addr()
                            .map(|addr| addr.to_string().magenta().to_string())
                            .unwrap_or_else(|_| "unknown".to_string())
                    );

                    return Ok(());
                }
            }
        }
    }

    pub async fn handle_packet(&self, owner: &mut EncryptedStream, packet: Packet) -> Result<()> {
        debug!(
            "handling packet from {}:\n{:?}",
            owner
                .peer_addr()
                .map(|addr| addr.to_string().magenta().to_string())
                .unwrap_or_else(|_| "unknown".to_string()),
            packet
        );

        match packet {
            Packet::DeployRequest(request) => {
                debug!("received deploy request: {:?}", request);
                owner
                    .send(Error {
                        message: "this daemon cannot deploy yet".into(),
                    })
                    .await?;
            }
            _ => {
                debug!("received unknown packet: {:?}", packet);
                // handle other packets here
            }
        }

        // handle the packet here
        Ok(())
    }
}
