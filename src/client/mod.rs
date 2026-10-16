use colored::Colorize;
use eyre::{Result, eyre};
use log::{debug, error};

use crate::{
    common::{
        proto::{HelloFinished, PROTOCOL_VERSION, Packet},
        stream::EncryptedStream,
    },
    config::ClientConfig,
};

pub struct CoeusClient {
    stream: EncryptedStream,
    config: ClientConfig,
}

impl CoeusClient {
    pub async fn new(config: ClientConfig) -> Result<Self> {
        let mut stream = EncryptedStream::connect(config.address(), &config.key).await?;

        // trade client and server hellos
        stream.send(Packet::HelloRequest(Box::default())).await?;

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
            Some(Packet::HelloResponse(inner)) => {
                debug!(
                    "received hello response from server {}: protocol_version={}, version={}",
                    stream
                        .peer_addr()
                        .map(|addr| addr.to_string().magenta().to_string())
                        .unwrap_or_else(|_| "unknown".to_string()),
                    inner.protocol_version,
                    inner.version
                );
                stream
                    .send(Packet::HelloFinished(Box::new(HelloFinished {
                        compatible: inner.protocol_version == PROTOCOL_VERSION,
                    })))
                    .await?;

                if inner.protocol_version != PROTOCOL_VERSION {
                    error!(
                        "server {} is incompatible with this client",
                        stream
                            .peer_addr()
                            .map(|addr| addr.to_string().magenta().to_string())
                            .unwrap_or_else(|_| "unknown".to_string())
                    );

                    return Err(eyre!("client is incompatible with this server"));
                }
            }
            _ => {
                return Err(eyre!("client sent unexpected packet during handshake"));
            }
        }

        debug!(
            "connected to coeus peer at {}",
            config.address().to_string().magenta()
        );

        Ok(Self { stream, config })
    }

    pub async fn send(&mut self, packet: impl Into<Packet>) -> Result<()> {
        self.stream.send(packet.into()).await?;
        Ok(())
    }
}
