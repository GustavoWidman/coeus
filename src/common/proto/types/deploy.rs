use serde::{Deserialize, Serialize};

use crate::common::proto::Packet;

#[derive(Debug, Serialize, Deserialize)]
pub struct DeployRequest {
    pub revision: String,
    pub dry_run: bool,
}

impl From<DeployRequest> for Packet {
    fn from(value: DeployRequest) -> Self {
        Packet::DeployRequest(Box::new(value))
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DeployAccepted {
    pub message: String,
}

impl From<DeployAccepted> for Packet {
    fn from(value: DeployAccepted) -> Self {
        Packet::DeployAccepted(Box::new(value))
    }
}
