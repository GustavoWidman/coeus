use colored::Colorize;
use eyre::{Result, eyre};
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

    /// Receive and handle the next packet from the server.
    ///
    /// The current protocol sends one acceptance packet for a deploy request and keeps
    /// the connection open, so this handles one packet instead of waiting for EOF.
    pub async fn listen(&mut self) -> Result<()> {
        let packet = self
            .stream
            .recv()
            .await?
            .ok_or_else(|| eyre!("server closed before sending a response"))?;
        self.handle_packet(packet).await
    }

    /// Send a request while listening for the server's response concurrently.
    pub async fn send_and_listen(&mut self, packet: impl Into<Packet>) -> Result<()> {
        let response = {
            let (reader, writer) = self.stream.split_mut();
            let receive_response = async {
                reader
                    .recv()
                    .await?
                    .ok_or_else(|| eyre!("server closed before sending a response"))
            };

            let (_, response) = tokio::try_join!(writer.send(packet), receive_response)?;
            response
        };

        self.handle_packet(response).await
    }

    pub async fn handle_packet(&self, packet: Packet) -> Result<()> {
        match packet {
            Packet::DeployAccepted(accepted) => {
                info!("deployment request accepted: {}", accepted.message);
                Ok(())
            }
            packet => Err(eyre!("unexpected server packet: {packet:?}")),
        }
    }

    pub fn split(self) -> (EncryptedReadHalf, EncryptedWriteHalf) {
        self.stream.split()
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use tokio::time::{Duration, timeout};

    use crate::common::{
        proto::{DeployAccepted, DeployRequest, Packet},
        stream::{EncryptedListener, EncryptedStream},
    };

    use super::CoeusClient;

    type TestResult<T = ()> = Result<T, Box<dyn Error + Send + Sync>>;

    const PSK: [u8; 32] = [0x42; 32];

    #[tokio::test]
    async fn sends_and_listens_concurrently_through_the_client_api() -> TestResult {
        timeout(Duration::from_secs(2), async {
            let listener = EncryptedListener::bind("127.0.0.1:0".parse()?, &PSK).await?;
            let address = listener.local_addr()?;

            let server = tokio::spawn(async move {
                let (mut stream, _) = listener.accept().await?;
                match stream.recv().await? {
                    Some(Packet::DeployRequest(request)) => {
                        assert_eq!(request.rev, "abc123");
                    }
                    other => panic!("unexpected request: {other:?}"),
                }
                stream
                    .send(Packet::DeployAccepted(Box::new(DeployAccepted {
                        message: "accepted".into(),
                    })))
                    .await?;
                Ok::<_, Box<dyn Error + Send + Sync>>(())
            });

            let stream = EncryptedStream::connect(address, &PSK).await?;
            let mut client = CoeusClient { stream };
            client
                .send_and_listen(DeployRequest {
                    rev: "abc123".into(),
                    dry_run: true,
                    clean_substituters: false,
                })
                .await?;
            server.await??;
            Ok::<_, Box<dyn Error + Send + Sync>>(())
        })
        .await??;
        Ok(())
    }
}
