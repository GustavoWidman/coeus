use std::fmt;

use serde::{Deserialize, Serialize};

use super::{Action, JobId, OrderId, Origin, Progress, Source, StorePath, Timestamp};

/// where a running job is, in pipeline order
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Phase {
    /// the runner started and is setting up
    Preparing,
    /// fetching the flake source
    Fetching,
    /// evaluating the system configuration
    Evaluating,
    /// building and downloading the system closure
    Building,
    /// switching the machine to the new system. the point of no return.
    Activating,
    /// checking the machine is healthy before keeping the new system
    Verifying,
}

impl Phase {
    /// everything before activation can be stopped and redone: the store
    /// keeps whatever was already built
    pub const fn is_cancellable(self) -> bool {
        (self as u8) < (Self::Activating as u8)
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Preparing => "preparing",
            Self::Fetching => "fetching",
            Self::Evaluating => "evaluating",
            Self::Building => "building",
            Self::Activating => "activating",
            Self::Verifying => "verifying",
        }
    }
}

impl fmt::Display for Phase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// the only shapes a job's life can take: a finished job has an outcome and
/// nothing else
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum JobState {
    Queued,
    Running { phase: Phase },
    Finished(Outcome),
}

impl JobState {
    pub const fn is_finished(&self) -> bool {
        matches!(self, Self::Finished(_))
    }

    pub const fn phase(&self) -> Option<Phase> {
        match self {
            Self::Running { phase } => Some(*phase),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Outcome {
    /// the machine reached the order's goal
    Succeeded,
    /// the machine already ran the system the order would build
    UpToDate,
    Failed(Failure),
    Cancelled,
    /// a newer order replaced this one before it activated
    Superseded { by: JobId },
    /// the runner died while activating and the machine did not converge
    Interrupted,
    /// activated, failed its health checks, and went back to the previous system
    RolledBack(Failure),
}

impl Outcome {
    /// whether the order did what it was meant to, or was rightly replaced
    pub const fn is_success(&self) -> bool {
        matches!(
            self,
            Self::Succeeded | Self::UpToDate | Self::Superseded { .. }
        )
    }

    pub fn failure(&self) -> Option<&Failure> {
        match self {
            Self::Failed(failure) | Self::RolledBack(failure) => Some(failure),
            _ => None,
        }
    }
}

/// why a job failed, with the end of the output that explains it
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Failure {
    pub phase: Phase,
    pub message: String,
    /// the last lines of output before the failure, at most [Failure::TAIL]
    pub tail: Vec<String>,
}

impl Failure {
    pub const TAIL: usize = 20;

    pub fn new(phase: Phase, message: impl Into<String>) -> Self {
        Self {
            phase,
            message: message.into(),
            tail: Vec::new(),
        }
    }
}

/// what clients see of a job
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobSummary {
    pub id: JobId,
    pub order: OrderId,
    pub source: Source,
    pub action: Action,
    pub origin: Origin,
    pub state: JobState,
    pub ordered_at: Timestamp,
    pub started_at: Option<Timestamp>,
    pub finished_at: Option<Timestamp>,
    /// the built system, once evaluation named it
    pub toplevel: Option<StorePath>,
    /// how many runners worked on it; above one after a restart
    pub attempt: u8,
}

impl JobSummary {
    /// time spent running so far, or in total once finished
    pub fn duration(&self) -> Option<std::time::Duration> {
        let started = self.started_at?;
        Some(match self.finished_at {
            Some(finished) => started.until(finished),
            None => started.elapsed(),
        })
    }
}

/// a running job as the live views need it
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobView {
    pub summary: JobSummary,
    pub progress: Progress,
    pub phase_started_at: Option<Timestamp>,
    /// how many packages change, once the closure diff is known
    pub changes: Option<u32>,
}

impl JobView {
    pub fn queued(summary: JobSummary) -> Self {
        Self {
            summary,
            progress: Progress::default(),
            phase_started_at: None,
            changes: None,
        }
    }
}
