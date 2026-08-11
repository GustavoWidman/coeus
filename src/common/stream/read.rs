use std::{
    pin::Pin,
    task::{Context, Poll},
};

use futures_util::Stream;
use tokio::io::{self, AsyncRead, ReadBuf};

use crate::common::stream::EncryptedStream;

impl AsyncRead for EncryptedStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        output: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        if output.remaining() == 0 {
            return Poll::Ready(Ok(()));
        }

        if !self.plaintext.is_empty() {
            let count = output.remaining().min(self.plaintext.len());
            output.put_slice(&self.plaintext.split_to(count));
            return Poll::Ready(Ok(()));
        }

        match Pin::new(&mut self.framed).poll_next(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(None) => Poll::Ready(Ok(())),
            Poll::Ready(Some(Err(error))) => Poll::Ready(Err(error)),
            Poll::Ready(Some(Ok(ciphertext))) => {
                let mut plaintext = vec![0u8; Self::MAX_FRAME];

                let length = self
                    .transport
                    .read_message(&ciphertext, &mut plaintext)
                    .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;

                if length == 0 {
                    return Poll::Ready(Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "empty encrypted frame",
                    )));
                }

                self.plaintext.extend_from_slice(&plaintext[..length]);

                let count = output.remaining().min(self.plaintext.len());
                output.put_slice(&self.plaintext.split_to(count));

                Poll::Ready(Ok(()))
            }
        }
    }
}
