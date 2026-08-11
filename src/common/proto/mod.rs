pub mod envelope;

#[cfg(test)]
mod test;

pub use coeus_proto_macros::{Packet, packet_enum};

pub const PROTOCOL_VERSION: u8 = 1;

#[packet_enum]
#[derive(Debug)]
pub enum Packet {
}
