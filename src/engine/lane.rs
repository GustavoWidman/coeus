use crate::common::model::{JobId, Phase, Placement};

/// the machine's deployment slots: at most one running job and at most one
/// waiting behind it. a waiting job without a running one cannot exist.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Lane {
    #[default]
    Idle,
    Busy {
        running: JobId,
        next: Option<JobId>,
    },
}

/// what admitting a job asks of the engine besides recording its placement
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Admitted {
    pub placement: Placement,
    /// start this job's runner now
    pub start: Option<JobId>,
    /// a queued job the new one replaced, to close as superseded
    pub replaced: Option<JobId>,
    /// the running job to stop because the new one preempts it
    pub preempt: Option<JobId>,
}

impl Lane {
    pub const fn running(self) -> Option<JobId> {
        match self {
            Self::Idle => None,
            Self::Busy { running, .. } => Some(running),
        }
    }

    pub const fn next(self) -> Option<JobId> {
        match self {
            Self::Idle => None,
            Self::Busy { next, .. } => next,
        }
    }

    /// places a new job. the newest order is what the machine should do next:
    /// it replaces whatever waited, and with `now` it also stops a running job
    /// that has not started activating.
    pub fn admit(&mut self, job: JobId, now: bool, running_phase: Option<Phase>) -> Admitted {
        match *self {
            Self::Idle => {
                *self = Self::Busy {
                    running: job,
                    next: None,
                };
                Admitted {
                    placement: Placement::Started,
                    start: Some(job),
                    replaced: None,
                    preempt: None,
                }
            }
            Self::Busy { running, next } => {
                *self = Self::Busy {
                    running,
                    next: Some(job),
                };

                let preemptible = now && running_phase.is_none_or(Phase::is_cancellable);
                Admitted {
                    placement: match preemptible {
                        true => Placement::Preempted { replaced: running },
                        false => Placement::Queued {
                            behind: running,
                            replaced: next,
                        },
                    },
                    start: None,
                    replaced: next,
                    preempt: preemptible.then_some(running),
                }
            }
        }
    }

    /// the running job finished; returns the waiting job, which runs now
    pub fn finish(&mut self, job: JobId) -> Option<JobId> {
        match *self {
            Self::Busy { running, next } if running == job => {
                *self = match next {
                    Some(next) => Self::Busy {
                        running: next,
                        next: None,
                    },
                    None => Self::Idle,
                };
                next
            }
            _ => None,
        }
    }

    /// takes a waiting job out of line
    pub fn withdraw(&mut self, job: JobId) -> bool {
        match *self {
            Self::Busy { running, next } if next == Some(job) => {
                *self = Self::Busy {
                    running,
                    next: None,
                };
                true
            }
            _ => false,
        }
    }
}
