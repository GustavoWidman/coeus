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

    pub async fn accept(&self) -> Result<(EncryptedStream, SocketAddr)> {
        let pending = self.accept_pending().await?;
        let address = pending.stream.peer_addr()?;
        Ok((pending.authenticate().await?, address))
    }

    /// Only TCP acceptance: cancellation cannot discard an in-flight handshake.
    pub(crate) async fn accept_pending(&self) -> io::Result<PendingConnection> {
        let (stream, _) = self.listener.accept().await?;
        Ok(PendingConnection {
            stream,
            psk: self.psk,
        })
    }
}

/// Own the accepted socket until authentication completes or its task is cancelled.
pub(crate) struct PendingConnection {
    stream: TcpStream,
    psk: [u8; 32],
}

impl PendingConnection {
    pub(crate) async fn authenticate(self) -> Result<EncryptedStream> {
        EncryptedStream::handshake(self.stream, &self.psk, false).await
    }
}
