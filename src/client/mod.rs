use colored::Colorize;
use eyre::{Result, eyre};
use log::debug;
use tokio::{net::lookup_host, time::timeout};

use crate::{
    common::{proto::Packet, stream::EncryptedStream},
    config::{ClientSettings, Endpoint, Key},
};

pub struct CoeusClient {
    stream: EncryptedStream,
}

impl CoeusClient {
    pub async fn connect(settings: &ClientSettings, endpoint: &Endpoint, key: &Key) -> Result<Self> {
        let attempt = async {
            let address = lookup_host((endpoint.host.as_str(), endpoint.port))
                .await?
                .next()
                .ok_or_else(|| eyre!("{} does not resolve", endpoint.host))?;

            EncryptedStream::connect(address, key.as_bytes()).await
        };

        let stream = timeout(settings.connect_timeout, attempt)
            .await
            .map_err(|_| eyre!("{} did not answer in time", endpoint.host))??;

        debug!("connected to {}", endpoint.host.magenta());
        Ok(Self { stream })
    }

    pub async fn send(&mut self, packet: impl Into<Packet>) -> Result<()> {
        self.stream.send(packet.into()).await
    }
}
