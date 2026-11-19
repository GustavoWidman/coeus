mod hello;
mod listener;
mod nonce;
mod packet;
mod read;
mod write;

#[cfg(test)]
mod test;

use colored::{ColoredString, Colorize};
use eyre::{Result, WrapErr, eyre};
pub use listener::EncryptedListener;
pub use listener::PendingConnection;
use snow::{Builder, HandshakeState, params::NoiseParams};
use socket2::{SockRef, TcpKeepalive};
use std::{net::SocketAddr, sync::Arc, time::Duration};
use tokio::{
    io::{self, AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};

use crate::common::proto::{HelloResponse, Role};
pub use read::EncryptedReadHalf;
pub use write::EncryptedWriteHalf;

pub struct EncryptedStream {
    reader: EncryptedReadHalf,
    writer: EncryptedWriteHalf,
}

impl EncryptedStream {
    const PATTERN: &'static str = "Noise_NNpsk0_25519_ChaChaPoly_BLAKE2s";
    const PROLOGUE: &'static [u8] = b"coeus-control/v1";
    const MAX_FRAME: usize = u16::MAX as usize;
    const TAG_LEN: usize = 16;

    /// authenticates over `stream` and runs the hello, returning the
    /// server's introduction along with the stream
    pub async fn connect(stream: TcpStream, psk: &[u8; 32]) -> Result<(Self, HelloResponse)> {
        // a server that cannot decrypt the first message hangs up, which
        // reads here as nothing more than the end of the stream
        let mut stream = Self::initiate(stream, psk)
            .await
            .wrap_err("the handshake failed; does the machine use this fleet key?")?;
        let hello = stream.greet().await?;
        Ok((stream, hello))
    }

    pub fn peer_addr(&self) -> io::Result<SocketAddr> {
        Ok(self.reader.peer_addr)
    }

    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        Ok(self.reader.local_addr)
    }

    /// the peer's address, styled for log lines
    pub fn peer(&self) -> ColoredString {
        self.reader.peer_addr.to_string().magenta()
    }

    pub fn split(&mut self) -> (&mut EncryptedReadHalf, &mut EncryptedWriteHalf) {
        (&mut self.reader, &mut self.writer)
    }

    fn builder(psk: &[u8; 32]) -> Result<Builder<'_>> {
        let params: NoiseParams = Self::PATTERN.parse()?;
        Ok(Builder::new(params).prologue(Self::PROLOGUE)?.psk(0, psk)?)
    }

    async fn initiate(stream: TcpStream, psk: &[u8; 32]) -> Result<Self> {
        let state = Self::builder(psk)?.build_initiator()?;
        Self::handshake(stream, state).await
    }

    /// psk0 mixes the key into the first message, so it decrypts under
    /// the client's key alone: the first key here that opens it names the
    /// connection's role
    async fn respond(mut stream: TcpStream, keys: &[(Role, [u8; 32])]) -> Result<(Self, Role)> {
        let first = Self::read_frame(&mut stream).await?;
        let mut payload = vec![0u8; Self::MAX_FRAME];

        for (role, psk) in keys {
            let mut state = Self::builder(psk)?.build_responder()?;
            if let Ok(length) = state.read_message(&first, &mut payload) {
                if length != 0 {
                    return Err(eyre!("unexpected handshake payload"));
                }
                return Ok((Self::handshake(stream, state).await?, *role));
            }
        }

        Err(eyre!(
            "the handshake opens under no key this daemon accepts"
        ))
    }

    /// a connection silent for 30 s is probed every 10 s, and closed after
    /// three probes go unanswered: a dead peer surfaces within a minute
    fn keepalive() -> TcpKeepalive {
        TcpKeepalive::new()
            .with_time(Duration::from_secs(30))
            .with_interval(Duration::from_secs(10))
            .with_retries(3)
    }

    async fn handshake(mut stream: TcpStream, mut state: HandshakeState) -> Result<Self> {
        stream.set_nodelay(true)?;
        // a peer that sleeps or drops off its network can vanish without
        // closing the connection, and a watch only ever reads, so without
        // the kernel's probes it would wait on a dead connection forever
        SockRef::from(&stream).set_tcp_keepalive(&Self::keepalive())?;

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
                    let incoming = Self::read_frame(&mut stream).await?;
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

    async fn read_frame(stream: &mut TcpStream) -> Result<Vec<u8>> {
        let length = stream.read_u16().await? as usize;
        let mut frame = vec![0u8; length];
        stream.read_exact(&mut frame).await?;
        Ok(frame)
    }
}
