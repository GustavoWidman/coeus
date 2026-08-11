use std::{
    pin::Pin,
    task::{Context, Poll},
};

use bytes::Bytes;
use futures_util::{Sink, ready};
use tokio::io::{self, AsyncWrite};

use crate::common::stream::EncryptedStream;

impl AsyncWrite for EncryptedStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        plaintext: &[u8],
    ) -> Poll<io::Result<usize>> {
        if plaintext.is_empty() {
            return Poll::Ready(Ok(0));
        }

        let count = plaintext.len().min(Self::MAX_FRAME - Self::TAG_LEN);

        ready!(Pin::new(&mut self.framed).poll_ready(cx))?;

        let mut ciphertext = vec![0u8; count + Self::TAG_LEN];
        let length = self
            .transport
            .write_message(&plaintext[..count], &mut ciphertext)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;

        ciphertext.truncate(length);

        Pin::new(&mut self.framed)
            .start_send(Bytes::from(ciphertext))
            .map_err(|error| io::Error::new(io::ErrorKind::BrokenPipe, error))?;

        Poll::Ready(Ok(count))
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.framed).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.framed).poll_close(cx)
    }
}
