use std::sync::Arc;

use colored::Colorize;
use eyre::Result;
use log::{debug, error, info};

use crate::{
    common::{
        proto::Packet,
        stream::{EncryptedListener, EncryptedStream},
    },
    config::ServerConfig,
};

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
        info!(
            "coeus server listening on {}",
            self.config.address().to_string().magenta()
        );

        let server = Arc::new(self);
        loop {
            let (stream, addr) = server.listener.accept().await?;

            debug!(
                "hamdling new connection from {}",
                addr.to_string().magenta()
            );

            tokio::spawn({
                let server = server.clone();
                async move {
                    if let Err(e) = server.handle_connection(stream).await {
                        error!("error handling connection:\n{}", e);
                    }
                }
            });
        }
    }

    pub async fn handle_connection(&self, mut stream: EncryptedStream) -> Result<()> {
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

            self.handle_packet(&stream, packet).await.inspect_err(|e| {
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
    }

    pub async fn handle_packet(&self, owner: &EncryptedStream, packet: Packet) -> Result<()> {
        debug!(
            "handling packet from {}: {:?}",
            owner
                .peer_addr()
                .map(|addr| addr.to_string().magenta().to_string())
                .unwrap_or_else(|_| "unknown".to_string()),
            packet
        );

        // handle the packet here
        Ok(())
    }
}
