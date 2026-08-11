use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct DeployRequest {
    pub revision: String,
    pub dry_run: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DeployAccepted {
    pub message: String,
}
