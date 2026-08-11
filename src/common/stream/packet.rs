use bytes::Bytes;
use eyre::{Result, eyre};
use futures_util::{SinkExt, StreamExt};

use crate::common::{
    proto::{Packet, envelope::PacketEnvelope},
    stream::EncryptedStream,
};

impl EncryptedStream {
    pub async fn send(&mut self, packet: Packet) -> Result<()> {
        let envelope = PacketEnvelope::new(packet);
        let plaintext = postcard::to_stdvec(&envelope)?;

        self.send_encrypted_frame(&plaintext).await
    }

    pub async fn recv(&mut self) -> Result<Packet> {
        let plaintext = self.recv_encrypted_frame().await?;
        let envelope: PacketEnvelope = postcard::from_bytes(&plaintext)?;

        envelope.validate()?;
        Ok(envelope.packet)
    }

    async fn send_encrypted_frame(&mut self, plaintext: &[u8]) -> Result<()> {
        if plaintext.len() > Self::MAX_FRAME - Self::TAG_LEN {
            return Err(eyre!("packet is too large"));
        }

        let mut ciphertext = vec![0u8; Self::MAX_FRAME];
        let length = self.transport.write_message(plaintext, &mut ciphertext)?;
        ciphertext.truncate(length);

        self.framed.send(Bytes::from(ciphertext)).await?;
        Ok(())
    }

    async fn recv_encrypted_frame(&mut self) -> Result<Vec<u8>> {
        let ciphertext = self
            .framed
            .next()
            .await
            .ok_or_else(|| eyre!("encrypted stream closed"))??;

        let mut plaintext = vec![0u8; Self::MAX_FRAME];
        let length = self.transport.read_message(&ciphertext, &mut plaintext)?;
        plaintext.truncate(length);

        Ok(plaintext)
    }
}
