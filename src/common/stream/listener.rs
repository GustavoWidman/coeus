use std::{net::SocketAddr, sync::Arc};

use eyre::Result;
use tokio::{
    io,
    net::{TcpListener, TcpStream},
};

use crate::common::{model::HostInfo, proto::Role, stream::EncryptedStream};

pub struct EncryptedListener {
    listener: TcpListener,
    keys: Arc<[(Role, [u8; 32])]>,
    host: Arc<HostInfo>,
}

impl EncryptedListener {
    /// binds `address`, introducing itself as `host` to every client and
    /// accepting each of `keys` with its role, tried in order
    pub async fn bind(
        address: SocketAddr,
        keys: Vec<(Role, [u8; 32])>,
        host: HostInfo,
    ) -> Result<Self> {
        Ok(Self {
            listener: TcpListener::bind(address).await?,
            keys: keys.into(),
            host: Arc::new(host),
        })
    }

    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.listener.local_addr()
    }

    /// listens until a new connection is accepted and authenticates it,
    /// returning the [EncryptedStream] and the role its key carries
    pub async fn accept(&self) -> Result<(EncryptedStream, Role)> {
        self.listen().await?.authenticate().await
    }

    /// listens for new connections and returns a [PendingConnection]
    /// which can be used to authenticate the connection
    pub async fn listen(&self) -> Result<PendingConnection> {
        let (stream, _) = self.listener.accept().await?;
        Ok(PendingConnection {
            stream,
            keys: self.keys.clone(),
            host: self.host.clone(),
        })
    }
}

pub struct PendingConnection {
    stream: TcpStream,
    keys: Arc<[(Role, [u8; 32])]>,
    host: Arc<HostInfo>,
}

impl PendingConnection {
    /// runs the NOISE handshake and the hello on an accepted connection
    pub async fn authenticate(self) -> Result<(EncryptedStream, Role)> {
        let (mut stream, role) = EncryptedStream::respond(self.stream, &self.keys).await?;
        stream.welcome(&self.host).await?;
        Ok((stream, role))
    }
}
