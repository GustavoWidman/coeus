use crate::{
    client::Link,
    common::model::{
        Admission, Hostname, JobId, JobState, JobSummary, JobView, MachineView, Outcome, Rejection,
    },
};

/// one machine on the board
#[derive(Clone, Debug)]
pub struct Row {
    pub host: Hostname,
    pub link: Link,
    /// the latest view, kept while reconnecting
    pub view: Option<MachineView>,
    pub ordering: Ordering,
}

/// what this invocation did on a machine
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ordering {
    /// nothing: the row follows whatever the machine does
    Watching,
    /// waits for its wave of the rollout
    Pending {
        wave: usize,
    },
    /// the rollout stopped before reaching it
    Held,
    Sending,
    Admitted(Admission),
    Rejected(Rejection),
    Unreachable(String),
}

/// where a row stands, derived from its ordering and its machine's view
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Status<'a> {
    Pending {
        wave: usize,
    },
    Held,
    Sending,
    /// watching, before the first view
    Connecting,
    Unreachable(&'a str),
    Rejected(&'a Rejection),
    /// watching a machine with no job yet
    Idle,
    /// admitted, before the machine's view shows the job
    Admitted(JobId),
    Queued(&'a JobSummary),
    Running(&'a JobView),
    Finished(&'a JobSummary, &'a Outcome),
}

/// how an ordered machine ended, for the summary and the exit code
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// succeeded, up to date, or rightly superseded
    Done,
    /// failed, cancelled, interrupted, or rolled back
    Failed,
    Rejected,
    Unreachable,
    /// never ordered: the rollout stopped first
    Held,
}

impl Row {
    pub const fn new(host: Hostname, ordering: Ordering) -> Self {
        Self {
            host,
            link: Link::Connecting,
            view: None,
            ordering,
        }
    }

    pub fn status(&self) -> Status<'_> {
        match &self.ordering {
            Ordering::Watching => self.current(),
            Ordering::Pending { wave } => Status::Pending { wave: *wave },
            Ordering::Held => Status::Held,
            Ordering::Sending => Status::Sending,
            Ordering::Admitted(admission) => self.job(admission.job),
            Ordering::Rejected(rejection) => Status::Rejected(rejection),
            Ordering::Unreachable(reason) => Status::Unreachable(reason),
        }
    }

    /// the job this row is about, once the machine's view names it
    pub fn summary(&self) -> Option<&JobSummary> {
        match self.status() {
            Status::Queued(summary) | Status::Finished(summary, _) => Some(summary),
            Status::Running(job) => Some(&job.summary),
            _ => None,
        }
    }

    pub fn verdict(&self) -> Option<Verdict> {
        match self.status() {
            Status::Finished(_, outcome) if outcome.is_success() => Some(Verdict::Done),
            Status::Finished(..) => Some(Verdict::Failed),
            Status::Rejected(_) => Some(Verdict::Rejected),
            Status::Unreachable(_) if self.ordering != Ordering::Watching => {
                Some(Verdict::Unreachable)
            }
            Status::Held => Some(Verdict::Held),
            _ => None,
        }
    }

    /// the connection dropped while the row still has something to follow
    pub fn is_reconnecting(&self) -> bool {
        self.view.is_some()
            && self.link != Link::Connected
            && !matches!(self.status(), Status::Finished(..) | Status::Rejected(_))
    }

    fn job(&self, id: JobId) -> Status<'_> {
        let Some(view) = &self.view else {
            return Status::Admitted(id);
        };

        if let Some(running) = view.running.as_ref().filter(|job| job.summary.id == id) {
            return Status::Running(running);
        }

        match view.job(id) {
            Some(summary) => match &summary.state {
                JobState::Finished(outcome) => Status::Finished(summary, outcome),
                _ => Status::Queued(summary),
            },
            None => Status::Admitted(id),
        }
    }

    fn current(&self) -> Status<'_> {
        let Some(view) = &self.view else {
            return match &self.link {
                Link::Unreachable(reason) => Status::Unreachable(reason),
                _ => Status::Connecting,
            };
        };

        if let Some(running) = &view.running {
            return Status::Running(running);
        }
        if let Some(next) = &view.next {
            return Status::Queued(next);
        }
        match view.recent.first() {
            Some(
                summary @ JobSummary {
                    state: JobState::Finished(outcome),
                    ..
                },
            ) => Status::Finished(summary, outcome),
            _ => Status::Idle,
        }
    }
}
