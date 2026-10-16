use std::{
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use bytes::Bytes;
use futures_util::{Sink, ready};
use snow::StatelessTransportState;
use tokio::{
    io::{self, AsyncWrite},
    net::tcp::OwnedWriteHalf,
};
use tokio_util::codec::{FramedWrite, LengthDelimitedCodec};

use crate::common::stream::{EncryptedStream, nonce::Nonce};

/// an owned send half with its own outbound NOISE nonce sequence.
pub struct EncryptedWriteHalf {
    pub(super) framed: FramedWrite<OwnedWriteHalf, LengthDelimitedCodec>,
    pub(super) transport: Arc<StatelessTransportState>,
    pub(super) nonce: Nonce,
}

impl EncryptedWriteHalf {
    pub fn new(stream: OwnedWriteHalf, transport: Arc<StatelessTransportState>) -> Self {
        let codec = LengthDelimitedCodec::builder()
            .length_field_type::<u16>()
            .max_frame_length(EncryptedStream::MAX_FRAME)
            .new_codec();

        Self {
            framed: FramedWrite::new(stream, codec),
            transport,
            nonce: Nonce::new(),
        }
    }
}

impl AsyncWrite for EncryptedWriteHalf {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        plaintext: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.as_mut().get_mut();

        if plaintext.is_empty() {
            return Poll::Ready(Ok(0));
        }

        let count = plaintext
            .len()
            .min(EncryptedStream::MAX_FRAME - EncryptedStream::TAG_LEN);

        ready!(Pin::new(&mut this.framed).poll_ready(cx))?;

        let mut ciphertext = vec![0u8; count + EncryptedStream::TAG_LEN];
        let length = this
            .transport
            .write_message(*this.nonce, &plaintext[..count], &mut ciphertext)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        this.nonce
            .advance()
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;

        ciphertext.truncate(length);

        Pin::new(&mut this.framed)
            .start_send(Bytes::from(ciphertext))
            .map_err(|error| io::Error::new(io::ErrorKind::BrokenPipe, error))?;

        Poll::Ready(Ok(count))
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.as_mut().get_mut().framed).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.as_mut().get_mut().framed).poll_close(cx)
    }
}

impl AsyncWrite for EncryptedStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        plaintext: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.as_mut().get_mut().writer).poll_write(cx, plaintext)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.as_mut().get_mut().writer).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.as_mut().get_mut().writer).poll_shutdown(cx)
    }
}
