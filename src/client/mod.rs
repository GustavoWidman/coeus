use colored::Colorize;
use eyre::{Result, eyre};
use log::{debug, error, info};

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
    /// The current daemon keeps the connection open after its deploy acknowledgement, so
    /// this handles one response instead of waiting for EOF.
    pub async fn listen(&mut self) -> Result<()> {
        let peer = self.peer_address();
        let packet = self.stream.recv().await.inspect_err(|error| {
            error!("error receiving packet from {peer}:\n{error}");
        })?;

        match packet {
            Some(packet) => {
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
        let peer = self.peer_address();
        debug!("handling packet from coeus server at {peer}:\n{packet:?}");

        match packet {
            Packet::DeployAccepted(accepted) => {
                info!("deployment request accepted: {}", accepted.message);
                Ok(())
            }
            Packet::DeployRequest(_) => Err(eyre!("received a deploy request from the server")),
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
    async fn listener_handles_response_while_server_keeps_connection_open() -> TestResult {
        timeout(Duration::from_secs(2), async {
            let listener = EncryptedListener::bind("127.0.0.1:0".parse()?, &PSK).await?;
            let address = listener.local_addr()?;
            let (release_server, server_release) = tokio::sync::oneshot::channel();

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
                let _ = server_release.await;
                Ok::<_, Box<dyn Error + Send + Sync>>(())
            });

            let stream = EncryptedStream::connect(address, &PSK).await?;
            let mut client = CoeusClient { stream };
            client
                .send(DeployRequest {
                    rev: "abc123".into(),
                    dry_run: true,
                    clean_substituters: false,
                })
                .await?;
            client.listen().await?;
            release_server
                .send(())
                .expect("server should still be waiting after its response");
            server.await??;
            Ok::<_, Box<dyn Error + Send + Sync>>(())
        })
        .await??;
        Ok(())
    }

    #[tokio::test]
    async fn listener_errors_if_server_closes_before_a_response() -> TestResult {
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
                drop(stream);
                Ok::<_, Box<dyn Error + Send + Sync>>(())
            });

            let stream = EncryptedStream::connect(address, &PSK).await?;
            let mut client = CoeusClient { stream };
            client
                .send(DeployRequest {
                    rev: "abc123".into(),
                    dry_run: true,
                    clean_substituters: false,
                })
                .await?;
            let error = client
                .listen()
                .await
                .expect_err("closing before a response must fail");
            assert!(
                error
                    .to_string()
                    .contains("closed before sending a response")
            );
            server.await??;
            Ok::<_, Box<dyn Error + Send + Sync>>(())
        })
        .await??;
        Ok(())
    }

    #[tokio::test]
    async fn sends_and_listens_concurrently_through_the_client_api() -> TestResult {
        timeout(Duration::from_secs(2), async {
            let listener = EncryptedListener::bind("127.0.0.1:0".parse()?, &PSK).await?;
            let address = listener.local_addr()?;

            let (release_server, server_release) = tokio::sync::oneshot::channel();
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
                let _ = server_release.await;
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
            release_server
                .send(())
                .expect("server should still be waiting after its response");
            server.await??;
            Ok::<_, Box<dyn Error + Send + Sync>>(())
        })
        .await??;
        Ok(())
    }
}
