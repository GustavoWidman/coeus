use std::{net::SocketAddr, sync::Arc};

use eyre::Result;
use tokio::{
    io,
    net::{TcpListener, TcpStream},
};

use crate::common::{model::HostInfo, stream::EncryptedStream};

pub struct EncryptedListener {
    listener: TcpListener,
    psk: [u8; 32],
    host: Arc<HostInfo>,
}

impl EncryptedListener {
    /// binds `address`, introducing itself as `host` to every client
    pub async fn bind(address: SocketAddr, psk: &[u8; 32], host: HostInfo) -> Result<Self> {
        Ok(Self {
            listener: TcpListener::bind(address).await?,
            psk: *psk,
            host: Arc::new(host),
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
            host: self.host.clone(),
        })
    }
}

pub struct PendingConnection {
    stream: TcpStream,
    psk: [u8; 32],
    host: Arc<HostInfo>,
}

impl PendingConnection {
    /// runs the NOISE handshake and the hello on an accepted connection
    pub async fn authenticate(self) -> Result<EncryptedStream> {
        let mut stream = EncryptedStream::handshake(self.stream, &self.psk, false).await?;
        stream.welcome(&self.host).await?;
        Ok(stream)
    }
}
