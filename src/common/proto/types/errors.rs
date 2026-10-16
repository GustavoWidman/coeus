use coeus_proto_macros::Packet;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Packet)]
pub struct Error {
    pub message: String,
}
