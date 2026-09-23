use std::{
    pin::Pin,
    task::{Context, Poll},
};

use futures_util::Stream;
use tokio::io::{self, AsyncRead, ReadBuf};

use crate::common::stream::{EncryptedReadHalf, EncryptedStream};

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
