use serde::{Deserialize, Serialize};

use crate::common::proto::Packet;

#[derive(Default, Debug, Serialize, Deserialize, Packet)]
pub struct HelloRequest {}

#[derive(Debug, Serialize, Deserialize, Packet)]
pub struct HelloResponse {
    pub protocol_version: u8,
    pub version: String,
}

#[derive(Debug, Serialize, Deserialize, Packet)]
pub struct HelloFinished {
    pub compatible: bool,
}
