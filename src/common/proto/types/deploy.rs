use serde::{Deserialize, Serialize};

use crate::common::{
    model::{Admission, CancelReport, JobSelector, Order, Pinned, Rejection},
    proto::Packet,
};

/// order a deployment
#[derive(Debug, Serialize, Deserialize, Packet)]
pub struct DeployRequest {
    pub order: Order,
}

#[derive(Debug, Serialize, Deserialize, Packet)]
pub struct DeployAccepted {
    pub admission: Admission,
}

#[derive(Debug, Serialize, Deserialize, Packet)]
pub struct DeployRejected {
    pub rejection: Rejection,
}

/// withdraw a queued job or stop a running one before it activates
#[derive(Debug, Serialize, Deserialize, Packet)]
pub struct CancelRequest {
    pub job: JobSelector,
}

#[derive(Debug, Serialize, Deserialize, Packet)]
pub struct CancelResponse {
    pub report: CancelReport,
}

/// pin a ref of the daemon's flake repository to a commit
#[derive(Debug, Serialize, Deserialize, Packet)]
pub struct ResolveRequest {
    pub git_ref: String,
}

#[derive(Debug, Serialize, Deserialize, Packet)]
pub struct ResolveResponse {
    pub pinned: Pinned,
}
