mod listener;
mod nonce;
mod packet;
mod read;
mod write;

#[cfg(test)]
mod test;

use eyre::{Result, eyre};
pub use listener::EncryptedListener;
pub use listener::PendingConnection;
use snow::{Builder, HandshakeState, params::NoiseParams};
use std::{net::SocketAddr, sync::Arc};
use tokio::{
    io::{self, AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};

use read::EncryptedReadHalf;
use write::EncryptedWriteHalf;

pub struct EncryptedStream {
    reader: EncryptedReadHalf,
    writer: EncryptedWriteHalf,
}

impl EncryptedStream {
    const PATTERN: &'static str = "Noise_NNpsk0_25519_ChaChaPoly_BLAKE2s";
    const PROLOGUE: &'static [u8] = b"coeus-control/v1";
    const MAX_FRAME: usize = u16::MAX as usize;
    const TAG_LEN: usize = 16;

    pub async fn connect(address: SocketAddr, psk: &[u8; 32]) -> Result<Self> {
        let stream = TcpStream::connect(address).await?;
        Self::handshake(stream, psk, true).await
    }

    pub async fn accept(address: SocketAddr, psk: &[u8; 32]) -> Result<(Self, SocketAddr)> {
        let listener = EncryptedListener::bind(address, psk).await?;
        listener.accept().await
    }

    pub fn peer_addr(&self) -> io::Result<SocketAddr> {
        Ok(self.reader.peer_addr)
    }

    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        Ok(self.reader.local_addr)
    }

    pub fn split(&mut self) -> (&mut EncryptedReadHalf, &mut EncryptedWriteHalf) {
        (&mut self.reader, &mut self.writer)
    }

    async fn handshake(mut stream: TcpStream, psk: &[u8; 32], initiator: bool) -> Result<Self> {
        stream.set_nodelay(true)?;

        let params: NoiseParams = Self::PATTERN.parse()?;
        let builder = Builder::new(params).prologue(Self::PROLOGUE)?.psk(0, psk)?;

        let mut state: HandshakeState = match initiator {
            true => builder.build_initiator()?,
            false => builder.build_responder()?,
        };

        let mut message = vec![0u8; Self::MAX_FRAME];
        let mut payload = vec![0u8; Self::MAX_FRAME];

        while !state.is_handshake_finished() {
            match state.is_my_turn() {
                true => {
                    let length = state.write_message(&[], &mut message)?;
                    stream.write_u16(length as u16).await?;
                    stream.write_all(&message[..length]).await?;
                }
                false => {
                    let length = stream.read_u16().await? as usize;
                    let mut incoming = vec![0u8; length];
                    stream.read_exact(&mut incoming).await?;

                    let payload_length = state.read_message(&incoming, &mut payload)?;

                    if payload_length != 0 {
                        return Err(eyre!("unexpected handshake payload"));
                    }
                }
            }
        }

        let transport = Arc::new(state.into_stateless_transport_mode()?);
        let (read_stream, write_stream) = stream.into_split();

        Ok(Self {
            reader: EncryptedReadHalf::new(read_stream, transport.clone())?,
            writer: EncryptedWriteHalf::new(write_stream, transport),
        })
    }
}
