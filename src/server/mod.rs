use std::sync::Arc;

use colored::Colorize;
use eyre::Result;
use log::{debug, error, info};
use tokio::sync::Mutex;

use crate::{
    common::{
        proto::{DeployAccepted, Packet},
        stream::{EncryptedListener, EncryptedStream},
    },
    config::ServerConfig,
    deploy::Deployer,
};

pub struct CoeusServer {
    listener: EncryptedListener,

    /// IMPORTANT:
    /// this is a [tokio::sync::Mutex] on purpose as it is a FIFO/"fair" mutex,
    /// which is important for proper ordering of deploy requests,
    /// as they are processed in the order they are received
    deployer: Mutex<Deployer>,

    config: ServerConfig,
}

impl CoeusServer {
    pub async fn new(config: ServerConfig) -> Result<Self> {
        let listener = EncryptedListener::bind(config.address(), &config.key).await?;
        let deployer = Deployer::new(config.clone()).await?.into();

        Ok(Self {
            listener,
            deployer,
            config,
        })
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
                let deployer = self.deployer.lock().await;

                owner
                    .send(DeployAccepted {
                        message: "deploy request accepted".into(),
                    })
                    .await?;

                deployer.deploy(request.as_ref()).await?;
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
