use clap::Args;
use serde::{Deserialize, Serialize};

use crate::common::proto::Packet;

#[derive(Args, Debug, Serialize, Deserialize)]
pub struct DeployRequest {
    #[arg(short, long, value_name = "REVISION", default_value = "main")]
    pub rev: String,

    /// Sets "eval-cache" to false in nix options
    #[arg(long, value_name = "DRY_RUN", default_value_t = false)]
    pub dry_run: bool,

    /// Ignore stale substituters from the current system
    #[arg(long, value_name = "CLEAN_SUBSTITUTERS", default_value_t = false)]
    pub clean_substituters: bool,
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
