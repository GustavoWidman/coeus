use std::net::SocketAddr;

use eyre::Result;
use tokio::{
    io,
    net::{TcpListener, TcpStream},
};

use crate::common::stream::EncryptedStream;

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
        EncryptedStream::handshake(self.stream, &self.psk, false).await
    }
}
