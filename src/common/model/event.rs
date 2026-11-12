use serde::{Deserialize, Serialize};

use super::{ClosureDiff, JobState, JobView, Outcome, Phase, Progress, StorePath, Timestamp};

/// what happened to a job, in order. the runner appends these to the job log;
/// the daemon and the CLI fold them into a [JobView] with [JobView::apply].
///
/// stored as JSON so a daemon built after a self-update still reads the logs
/// its predecessor's runners wrote.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobEvent {
    /// a runner took the job; a job resumed after its runner died starts the
    /// next attempt
    Started {
        attempt: u8,
        pid: u32,
        version: String,
    },
    Phase(Phase),
    Evaluated {
        drv: StorePath,
        toplevel: StorePath,
    },
    Progress(Progress),
    Diff(ClosureDiff),
    /// a message for people, e.g. "the system already runs this generation"
    Note(String),
    Finished(Outcome),
}

/// an event with the time it was recorded
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stamped<T> {
    pub at: Timestamp,
    pub event: T,
}

impl<T> Stamped<T> {
    pub fn now(event: T) -> Self {
        Self {
            at: Timestamp::now(),
            event,
        }
    }
}

impl JobView {
    /// the only way a job view changes
    pub fn apply(&mut self, stamped: &Stamped<JobEvent>) {
        let at = stamped.at;
        let summary = &mut self.summary;

        match &stamped.event {
            JobEvent::Started { attempt, .. } => {
                summary.attempt = *attempt;
                summary.started_at.get_or_insert(at);
                summary.state = JobState::Running {
                    phase: Phase::Preparing,
                };
                self.phase_started_at = Some(at);
            }
            JobEvent::Phase(phase) => {
                summary.state = JobState::Running { phase: *phase };
                self.phase_started_at = Some(at);
            }
            JobEvent::Evaluated { toplevel, .. } => summary.toplevel = Some(toplevel.clone()),
            JobEvent::Progress(progress) => self.progress = progress.clone(),
            JobEvent::Diff(diff) => self.changes = Some(diff.changes.len() as u32),
            JobEvent::Note(_) => {}
            JobEvent::Finished(outcome) => {
                summary.state = JobState::Finished(outcome.clone());
                summary.finished_at = Some(at);
            }
        }
    }
}
