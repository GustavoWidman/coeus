use std::net::SocketAddr;

use eyre::Result;
use tokio::{io, net::TcpListener};

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
        let (stream, addr) = self.listener.accept().await?;

        Ok((
            EncryptedStream::handshake(stream, &self.psk, false).await?,
            addr,
        ))
    }
}
