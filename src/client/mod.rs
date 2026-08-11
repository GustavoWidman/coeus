use colored::Colorize;
use eyre::Result;
use log::info;

use crate::{
    common::{proto::Packet, stream::EncryptedStream},
    config::ClientConfig,
};

pub struct CoeusClient {
    stream: EncryptedStream,
    config: ClientConfig,
}

impl CoeusClient {
    pub async fn new(config: ClientConfig) -> Result<Self> {
        let stream = EncryptedStream::connect(config.address(), &config.key).await?;

        info!(
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
