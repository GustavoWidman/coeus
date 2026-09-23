use std::{
    pin::Pin,
    task::{Context, Poll},
};

use bytes::Bytes;
use futures_util::{Sink, ready};
use tokio::io::{self, AsyncWrite};

use crate::common::stream::{EncryptedStream, EncryptedWriteHalf, advance_nonce};

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
            .write_message(this.nonce, &plaintext[..count], &mut ciphertext)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        advance_nonce(&mut this.nonce)
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
