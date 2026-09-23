use colored::Colorize;
use eyre::Result;
use log::info;

use crate::{
    common::{
        proto::Packet,
        stream::{EncryptedReadHalf, EncryptedStream, EncryptedWriteHalf},
    },
    config::ClientConfig,
};

pub struct CoeusClient {
    stream: EncryptedStream,
}

impl CoeusClient {
    pub async fn new(config: ClientConfig) -> Result<Self> {
        let stream = EncryptedStream::connect(config.address(), &config.key).await?;

        info!(
            "connected to coeus peer at {}",
            config.address().to_string().magenta()
        );

        Ok(Self { stream })
    }

    pub async fn send(&mut self, packet: impl Into<Packet>) -> Result<()> {
        self.stream.send(packet.into()).await?;
        Ok(())
    }

    pub async fn recv(&mut self) -> Result<Option<Packet>> {
        self.stream.recv().await
    }

    /// Consume the client and expose concurrent receive and send halves.
    pub fn split(self) -> (EncryptedReadHalf, EncryptedWriteHalf) {
        self.stream.split()
    }
}
