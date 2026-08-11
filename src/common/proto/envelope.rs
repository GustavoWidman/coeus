use serde::{Deserialize, Serialize};

use crate::common::proto::{PROTOCOL_VERSION, Packet};

#[derive(Debug, Serialize, Deserialize)]
pub struct PacketEnvelope {
    pub protocol_version: u8,
    pub packet: Packet,
}

impl PacketEnvelope {
    pub fn new(packet: Packet) -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            packet,
        }
    }

    pub fn validate(&self) -> eyre::Result<()> {
        if self.protocol_version != PROTOCOL_VERSION {
            return Err(eyre::eyre!(
                "unsupported protocol version: {}",
                self.protocol_version
            ));
        }

        Ok(())
    }
}
