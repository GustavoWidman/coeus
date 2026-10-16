use std::{
    net::SocketAddr,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use bytes::BytesMut;
use eyre::Result;
use futures_util::Stream;
use snow::StatelessTransportState;
use tokio::{
    io::{self, AsyncRead, ReadBuf},
    net::tcp::OwnedReadHalf,
};
use tokio_util::codec::{FramedRead, LengthDelimitedCodec};

use crate::common::stream::{EncryptedStream, nonce::Nonce};

/// an owned receive half with its own inbound NOISE nonce sequence.
pub struct EncryptedReadHalf {
    pub peer_addr: SocketAddr,
    pub local_addr: SocketAddr,
    pub(super) framed: FramedRead<OwnedReadHalf, LengthDelimitedCodec>,
    pub(super) transport: Arc<StatelessTransportState>,
    pub(super) nonce: Nonce,
    plaintext: BytesMut,
}

impl EncryptedReadHalf {
    pub fn new(stream: OwnedReadHalf, transport: Arc<StatelessTransportState>) -> Result<Self> {
        let codec = LengthDelimitedCodec::builder()
            .length_field_type::<u16>()
            .max_frame_length(EncryptedStream::MAX_FRAME)
            .new_codec();

        Ok(Self {
            peer_addr: stream.peer_addr()?,
            local_addr: stream.local_addr()?,
            framed: FramedRead::new(stream, codec),
            transport,
            plaintext: BytesMut::new(),
            nonce: Nonce::new(),
        })
    }
}

impl AsyncRead for EncryptedReadHalf {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        output: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.as_mut().get_mut();

        if output.remaining() == 0 {
            return Poll::Ready(Ok(()));
        }

        if !this.plaintext.is_empty() {
            let count = output.remaining().min(this.plaintext.len());
            output.put_slice(&this.plaintext.split_to(count));
            return Poll::Ready(Ok(()));
        }

        match Pin::new(&mut this.framed).poll_next(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(None) => Poll::Ready(Ok(())),
            Poll::Ready(Some(Err(error))) => Poll::Ready(Err(error)),
            Poll::Ready(Some(Ok(ciphertext))) => {
                let plaintext = this
                    .decrypt_frame(&ciphertext)
                    .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;

                if plaintext.is_empty() {
                    return Poll::Ready(Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "empty encrypted frame",
                    )));
                }

                this.plaintext.extend_from_slice(&plaintext);

                let count = output.remaining().min(this.plaintext.len());
                output.put_slice(&this.plaintext.split_to(count));

                Poll::Ready(Ok(()))
            }
        }
    }
}

impl AsyncRead for EncryptedStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        output: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.as_mut().get_mut().reader).poll_read(cx, output)
    }
}
