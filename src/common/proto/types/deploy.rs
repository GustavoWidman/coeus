use clap::Args;
use serde::{Deserialize, Serialize};

use crate::common::proto::Packet;

#[derive(Args, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeployRequest {
    /// Reuse this nonce with identical options after a lost ACK; never generate a new ID for a retry.
    #[arg(long, default_value_t = 0)]
    pub request_id: u64,

    #[arg(short, long, value_name = "REVISION", default_value = "main")]
    pub rev: String,

    /// Build only; never activate. Retains the exact closure for inspection.
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
    pub job_id: super::JobId,
    pub message: String,
}

impl From<DeployAccepted> for Packet {
    fn from(value: DeployAccepted) -> Self {
        Packet::DeployAccepted(Box::new(value))
    }
}
