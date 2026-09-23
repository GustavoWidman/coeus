use std::{future::poll_fn, pin::Pin};

use bytes::Bytes;
use eyre::{Result, eyre};
use futures_util::{Sink, SinkExt, StreamExt};

use crate::common::proto::{Packet, envelope::PacketEnvelope};

use super::{EncryptedReadHalf, EncryptedStream, EncryptedWriteHalf, advance_nonce};

impl EncryptedStream {
    pub async fn send(&mut self, packet: impl Into<Packet>) -> Result<()> {
        self.writer.send(packet).await
    }

    pub async fn recv(&mut self) -> Result<Option<Packet>> {
        self.reader.recv().await
    }
}

impl EncryptedWriteHalf {
    pub async fn send(&mut self, packet: impl Into<Packet>) -> Result<()> {
        let envelope = PacketEnvelope::new(packet.into());
        let plaintext = postcard::to_stdvec(&envelope)?;

        self.send_encrypted_frame(&plaintext).await
    }

    async fn send_encrypted_frame(&mut self, plaintext: &[u8]) -> Result<()> {
        if plaintext.len() > EncryptedStream::MAX_FRAME - EncryptedStream::TAG_LEN {
            return Err(eyre!("packet is too large"));
        }

        poll_fn(|cx| Pin::new(&mut self.framed).poll_ready(cx)).await?;

        let mut ciphertext = vec![0u8; EncryptedStream::MAX_FRAME];
        let length = self
            .transport
            .write_message(self.nonce, plaintext, &mut ciphertext)?;
        advance_nonce(&mut self.nonce)?;
        ciphertext.truncate(length);

        Pin::new(&mut self.framed).start_send(Bytes::from(ciphertext))?;
        self.framed.flush().await?;
        Ok(())
    }
}

impl EncryptedReadHalf {
    pub async fn recv(&mut self) -> Result<Option<Packet>> {
        let Some(ciphertext) = self.framed.next().await else {
            return Ok(None);
        };

        let ciphertext = ciphertext?;
        let plaintext = self.decrypt_frame(&ciphertext)?;
        let envelope: PacketEnvelope = postcard::from_bytes(&plaintext)?;
        envelope.validate()?;

        Ok(Some(envelope.packet))
    }

    pub(super) fn decrypt_frame(&mut self, ciphertext: &[u8]) -> Result<Vec<u8>> {
        let mut plaintext = vec![0u8; EncryptedStream::MAX_FRAME];
        let length = self
            .transport
            .read_message(self.nonce, ciphertext, &mut plaintext)?;
        advance_nonce(&mut self.nonce)?;
        plaintext.truncate(length);
        Ok(plaintext)
    }
}
