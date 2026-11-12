use serde::{Deserialize, Serialize};

use crate::common::{model::Generation, proto::Packet};

#[derive(Debug, Serialize, Deserialize, Packet)]
pub struct GenerationsRequest {}

#[derive(Debug, Serialize, Deserialize, Packet)]
pub struct GenerationsResponse {
    /// newest first
    pub generations: Vec<Generation>,
}

/// reboot the machine once no job is activating
#[derive(Debug, Serialize, Deserialize, Packet)]
pub struct RebootRequest {}

#[derive(Debug, Serialize, Deserialize, Packet)]
pub struct Ack {}
