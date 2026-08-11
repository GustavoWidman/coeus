pub mod envelope;

#[cfg(test)]
mod test;

pub use coeus_proto_macros::{Packet, packet_enum};

pub const PROTOCOL_VERSION: u8 = 1;

pub mod types;
pub use types::{DeployAccepted, DeployRequest};

#[packet_enum]
#[derive(Debug)]
pub enum Packet {
    #[packet(id = 0x0100, type = DeployRequest)]
    DeployRequest,

    #[packet(id = 0x0101, type = DeployAccepted)]
    DeployAccepted,
}
