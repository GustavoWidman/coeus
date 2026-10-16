use std::net::SocketAddr;

use colored::Colorize;
use eyre::{Result, eyre};
use log::error;
use tokio::{
    io,
    net::{TcpListener, TcpStream},
};

use crate::common::{
    proto::{HelloResponse, PROTOCOL_VERSION, Packet},
    stream::EncryptedStream,
};

pub struct EncryptedListener {
    listener: TcpListener,
    psk: [u8; 32],
}

impl EncryptedListener {
    pub async fn bind(address: SocketAddr, psk: &[u8; 32]) -> Result<Self> {
        Ok(Self {
            listener: TcpListener::bind(address).await?,
            psk: *psk,
        })
    }

    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.listener.local_addr()
    }

    /// listens until a new connection is accepted and authenticates it,
    /// returning the [EncryptedStream] and the peer's [SocketAddr]
    pub async fn accept(&self) -> Result<(EncryptedStream, SocketAddr)> {
        let pending = self.listen().await?;
        let address = pending.stream.peer_addr()?;
        Ok((pending.authenticate().await?, address))
    }

    /// listens for new connections and returns a [PendingConnection]
    /// which can be used to authenticate the connection
    pub async fn listen(&self) -> Result<PendingConnection> {
        let (stream, _) = self.listener.accept().await?;
        Ok(PendingConnection {
            stream,
            psk: self.psk,
        })
    }
}

pub struct PendingConnection {
    stream: TcpStream,
    psk: [u8; 32],
}

impl PendingConnection {
    pub async fn authenticate(self) -> Result<EncryptedStream> {
        let mut stream = EncryptedStream::handshake(self.stream, &self.psk, false).await?;

        // trade client and server hellos
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
            Some(Packet::HelloRequest(_)) => {
                stream
                    .send(Packet::HelloResponse(Box::new(HelloResponse {
                        protocol_version: PROTOCOL_VERSION,
                        version: env!("CARGO_PKG_VERSION").into(),
                    })))
                    .await?;
            }
            _ => {
                return Err(eyre!("client sent unexpected packet during handshake"));
            }
        }

        // await client compatibility statement
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
            Some(Packet::HelloFinished(inner)) => {
                if !inner.compatible {
                    error!(
                        "client {} is incompatible with this server",
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

        Ok(stream)
    }
}
