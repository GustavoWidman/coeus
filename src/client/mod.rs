mod commands;
mod fleet;
pub mod target;
mod tui;
pub use fleet::run;
mod terminal;
#[cfg(unix)]
mod terminal_output;
use colored::Colorize;
use eyre::{Result, eyre};
use log::{debug, error, info};

use crate::common::{
    proto::Packet,
    stream::{EncryptedReadHalf, EncryptedStream, EncryptedWriteHalf},
};

#[derive(Debug)]
pub struct EndpointUnreachable(pub std::net::SocketAddr);
impl std::fmt::Display for EndpointUnreachable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "endpoint {} unreachable", self.0)
    }
}
impl std::error::Error for EndpointUnreachable {}

pub struct CoeusClient {
    stream: EncryptedStream,
    usable: bool,
}

impl CoeusClient {
    /// Authenticate before any request or bootstrap decision. A live peer with a bad
    /// key/protocol is NEVER classified as unreachable.
    pub async fn new(address: std::net::SocketAddr, key: &[u8; 32]) -> Result<Self> {
        let tcp = tokio::time::timeout(
            std::time::Duration::from_secs(3),
            tokio::net::TcpStream::connect(address),
        )
        .await
        .map_err(|_| EndpointUnreachable(address))?
        .map_err(|_| EndpointUnreachable(address))?;
        let stream = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            EncryptedStream::initiate(tcp, key),
        )
        .await
        .map_err(|_| {
            eyre!("Noise handshake deadline exceeded; endpoint exists, refusing bootstrap")
        })??;
        let mut client = Self {
            stream,
            usable: true,
        };
        let crate::common::proto::Packet::HelloResponse(hello) = client
            .request(crate::common::proto::HelloRequest {})
            .await?
        else {
            return Err(eyre!(
                "unexpected authenticated hello response; refusing bootstrap"
            ));
        };
        eyre::ensure!(
            hello.protocol_version == crate::common::proto::PROTOCOL_VERSION,
            "incompatible daemon protocol; refusing bootstrap"
        );
        Ok(client)
    }

    #[cfg(test)]
    async fn connect(
        address: std::net::SocketAddr,
        key: &[u8; 32],
        deadline: std::time::Duration,
    ) -> Result<EncryptedStream> {
        tokio::time::timeout(deadline, EncryptedStream::connect(address, key))
            .await
            .map_err(|_| eyre!("connection/Noise handshake deadline exceeded"))?
    }

    pub async fn send(&mut self, packet: impl Into<Packet>) -> Result<()> {
        if !self.usable {
            return Err(eyre!("connection unusable; reconnect"));
        }
        self.usable = false;
        self.stream.send(packet.into()).await?;
        self.usable = true;
        Ok(())
    }

    /// Receive and handle the next packet from the server.
    ///
    /// The current daemon keeps the connection open after its deploy acknowledgement, so
    /// this handles one response instead of waiting for EOF.
    pub async fn listen(&mut self) -> Result<()> {
        if !self.usable {
            return Err(eyre!("connection unusable; reconnect"));
        }
        self.usable = false;
        let peer = self.peer_address();
        let packet = self.stream.recv().await.inspect_err(|error| {
            error!("error receiving packet from {peer}:\n{error}");
        })?;

        match packet {
            Some(packet) => {
                self.usable = true;
                self.handle_packet(packet).await.inspect_err(|error| {
                    error!("error handling packet from {peer}:\n{error}");
                })?;
            }
            None => {
                debug!("connection from {peer} closed before sending a response");
                return Err(eyre!("server closed before sending a response"));
            }
        }
        Ok(())
    }

    /// One outstanding request per connection; framing is cancelled only by closing
    /// the connection if the response deadline expires.
    pub async fn request(&mut self, packet: impl Into<Packet>) -> Result<Packet> {
        if !self.usable {
            return Err(eyre!(
                "connection is unusable; reconnect without blindly resubmitting a deployment"
            ));
        }
        // Poison before the await too: a caller may cancel/drop this future at
        // any point, leaving a partial frame or an unmatched server reply.
        self.usable = false;
        let result = tokio::time::timeout(std::time::Duration::from_secs(15), async {
            let (reader, writer) = self.stream.split_mut();
            let receive = async {
                reader
                    .recv()
                    .await?
                    .ok_or_else(|| eyre!("server closed before sending a response"))
            };
            let (_, response) = tokio::try_join!(writer.send(packet), receive)?;
            Ok::<_, eyre::Report>(response)
        })
        .await;
        match result {
            Ok(Ok(Packet::RequestRejected(rejected))) => {
                self.usable = true;
                Err(eyre!("request rejected: {}", rejected.message))
            }
            Ok(Ok(response)) => {
                self.usable = true;
                Ok(response)
            }
            Ok(Err(error)) => {
                self.usable = false;
                Err(error)
            }
            Err(_) => {
                self.usable = false;
                Err(eyre!(
                    "response deadline exceeded; acceptance may have happened; reconnect and query status"
                ))
            }
        }
    }

    /// Send a request while listening for the server's response concurrently.
    pub async fn send_and_listen(&mut self, packet: impl Into<Packet>) -> Result<()> {
        let response = self.request(packet).await?;
        self.handle_packet(response).await
    }

    pub async fn handle_packet(&self, packet: Packet) -> Result<()> {
        let peer = self.peer_address();
        debug!("handling packet from coeus server at {peer}:\n{packet:?}");

        match packet {
            Packet::DeployAccepted(accepted) => {
                info!(
                    "deployment {} accepted: {}",
                    accepted.job_id, accepted.message
                );
                Ok(())
            }
            _ => Err(eyre!("unexpected response from the server")),
        }
    }

    fn peer_address(&self) -> String {
        self.stream
            .peer_addr()
            .map(|address| address.to_string().magenta().to_string())
            .unwrap_or_else(|_| "unknown".to_string())
    }

    pub fn split(self) -> (EncryptedReadHalf, EncryptedWriteHalf) {
        self.stream.split()
    }
}

#[cfg(test)]
mod tests;
