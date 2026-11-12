use serde::{Deserialize, Serialize};

use crate::common::model::{JobId, Outcome};

/// coeusd's request for a runner to stop, left in the job directory
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum StopRequest {
    Cancel,
    Superseded { by: JobId },
}

impl StopRequest {
    /// the outcome a job stopped by this request finishes with
    pub const fn outcome(self) -> Outcome {
        match self {
            Self::Cancel => Outcome::Cancelled,
            Self::Superseded { by } => Outcome::Superseded { by },
        }
    }
}
