mod listener;
mod packet;
mod read;
mod write;

#[cfg(test)]
mod test;

use bytes::BytesMut;
use eyre::{Result, eyre};
pub use listener::EncryptedListener;
use snow::{Builder, HandshakeState, StatelessTransportState, params::NoiseParams};
use std::{net::SocketAddr, sync::Arc};
use tokio::{
    io::{self, AsyncReadExt, AsyncWriteExt},
    net::{
        TcpStream,
        tcp::{OwnedReadHalf, OwnedWriteHalf},
    },
};
use tokio_util::codec::{FramedRead, FramedWrite, LengthDelimitedCodec};

pub struct EncryptedStream {
    reader: EncryptedReadHalf,
    writer: EncryptedWriteHalf,
}

/// An owned receive half with its own inbound Noise nonce sequence.
pub struct EncryptedReadHalf {
    framed: FramedRead<OwnedReadHalf, LengthDelimitedCodec>,
    transport: Arc<StatelessTransportState>,
    nonce: u64,
    plaintext: BytesMut,
    peer_addr: SocketAddr,
    local_addr: SocketAddr,
}

/// An owned send half with its own outbound Noise nonce sequence.
pub struct EncryptedWriteHalf {
    framed: FramedWrite<OwnedWriteHalf, LengthDelimitedCodec>,
    transport: Arc<StatelessTransportState>,
    nonce: u64,
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

    /// Consume the stream and return independently owned receive and send halves.
    pub fn split(self) -> (EncryptedReadHalf, EncryptedWriteHalf) {
        (self.reader, self.writer)
    }

    pub(crate) fn split_mut(&mut self) -> (&mut EncryptedReadHalf, &mut EncryptedWriteHalf) {
        (&mut self.reader, &mut self.writer)
    }

    async fn handshake(mut stream: TcpStream, psk: &[u8; 32], initiator: bool) -> Result<Self> {
        stream.set_nodelay(true)?;
        let peer_addr = stream.peer_addr()?;
        let local_addr = stream.local_addr()?;

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

        // Snow derives independent initiator and responder cipher states at the Noise
        // Split() step. StatelessTransportState leaves nonce sequencing to each owned
        // half, so reads and writes can use those states concurrently without a lock.
        // Coeus does not currently rekey transport states; add coordinated directional
        // rekeying before introducing a rekey policy.
        let transport = Arc::new(state.into_stateless_transport_mode()?);
        let (read_stream, write_stream) = stream.into_split();

        Ok(Self {
            reader: EncryptedReadHalf {
                framed: FramedRead::new(read_stream, codec()),
                transport: Arc::clone(&transport),
                nonce: 0,
                plaintext: BytesMut::new(),
                peer_addr,
                local_addr,
            },
            writer: EncryptedWriteHalf {
                framed: FramedWrite::new(write_stream, codec()),
                transport,
                nonce: 0,
            },
        })
    }
}

pub(super) fn advance_nonce(nonce: &mut u64) -> Result<()> {
    *nonce = nonce
        .checked_add(1)
        .ok_or_else(|| eyre!("Noise transport nonce exhausted"))?;
    Ok(())
}

fn codec() -> LengthDelimitedCodec {
    LengthDelimitedCodec::builder()
        .length_field_type::<u16>()
        .max_frame_length(EncryptedStream::MAX_FRAME)
        .new_codec()
}
