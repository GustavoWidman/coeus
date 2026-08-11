mod listener;
mod packet;
mod read;
mod write;

#[cfg(test)]
mod test;

use bytes::BytesMut;
use eyre::{Result, eyre};
pub use listener::EncryptedListener;
use snow::{Builder, HandshakeState, TransportState, params::NoiseParams};
use std::net::SocketAddr;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};
use tokio_util::codec::{Framed, LengthDelimitedCodec};

pub struct EncryptedStream {
    framed: Framed<TcpStream, LengthDelimitedCodec>,
    transport: TransportState,
    plaintext: BytesMut,
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

        let codec = LengthDelimitedCodec::builder()
            .length_field_type::<u16>()
            .max_frame_length(Self::MAX_FRAME)
            .new_codec();

        Ok(Self {
            framed: Framed::new(stream, codec),
            transport: state.into_transport_mode()?,
            plaintext: BytesMut::new(),
        })
    }
}
