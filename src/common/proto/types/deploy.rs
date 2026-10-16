use clap::Args;
use serde::{Deserialize, Serialize};

use crate::common::proto::Packet;

#[derive(Args, Debug, Serialize, Deserialize, Packet)]
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

#[derive(Debug, Serialize, Deserialize, Packet)]
pub struct DeployAccepted {
    pub message: String,
}
