use colored::Colorize;
use eyre::{Result, eyre};
use log::debug;
use tokio::{net::lookup_host, time::timeout};

use crate::{
    common::{model::HostInfo, proto::Packet, stream::EncryptedStream},
    config::{ClientSettings, Endpoint, Key},
};

pub struct CoeusClient {
    stream: EncryptedStream,
    host: HostInfo,
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

        let (stream, hello) = timeout(settings.connect_timeout, attempt)
            .await
            .map_err(|_| eyre!("{} did not answer in time", endpoint.host))??;

        debug!("connected to {}", hello.host.hostname.to_string().magenta());
        Ok(Self {
            stream,
            host: hello.host,
        })
    }

    /// the machine on the other end, as its daemon introduced itself
    pub const fn host(&self) -> &HostInfo {
        &self.host
    }

    pub async fn send(&mut self, packet: impl Into<Packet>) -> Result<()> {
        self.stream.send(packet.into()).await
    }
}
