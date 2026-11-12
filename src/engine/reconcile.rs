use crate::{
    common::model::{JobId, JobState, Outcome, StorePath},
    job::StopRequest,
};

/// what coeusd finds of an unfinished job when it starts
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Observed {
    pub id: JobId,
    pub state: JobState,
    /// a live runner holds the job's log
    pub held: bool,
    pub stop: Option<StopRequest>,
    pub toplevel: Option<StorePath>,
}

/// what to do with the jobs found at startup
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Plan {
    /// the job that runs, and whether its runner has to be started again
    pub running: Option<(JobId, Resume)>,
    pub next: Option<JobId>,
    /// jobs to finish, with the outcome to record
    pub close: Vec<(JobId, Outcome)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resume {
    /// its runner outlived the previous coeusd: keep following its log
    Follow,
    /// start a runner for it, as a new attempt
    Spawn,
}

impl Plan {
    /// decides from what is on disk and what the machine runs. jobs come
    /// oldest first. an activation is never retried without a person: a job
    /// whose runner died while activating succeeded only if the machine runs
    /// what it built.
    pub fn reconcile(jobs: Vec<Observed>, current: Option<&StorePath>) -> Self {
        let mut plan = Self::default();
        let mut pending = Vec::new();

        for job in jobs {
            let phase = match &job.state {
                JobState::Finished(_) => continue,
                JobState::Queued => {
                    pending.push(job.id);
                    continue;
                }
                JobState::Running { phase } => *phase,
            };

            let verdict = match (job.held, job.stop) {
                (true, _) => Verdict::Run(Resume::Follow),
                (false, Some(stop)) => Verdict::Close(stop.outcome()),
                (false, None) if phase.is_cancellable() => Verdict::Run(Resume::Spawn),
                (false, None) => match (current, &job.toplevel) {
                    (Some(current), Some(toplevel)) if current == toplevel => {
                        Verdict::Close(Outcome::Succeeded)
                    }
                    _ => Verdict::Close(Outcome::Interrupted),
                },
            };

            match verdict {
                Verdict::Close(outcome) => plan.close.push((job.id, outcome)),
                Verdict::Run(resume) => {
                    if let Some((older, _)) = plan.running.replace((job.id, resume)) {
                        plan.close.push((older, Outcome::Interrupted));
                    }
                }
            }
        }

        let newest = pending.pop();
        for older in pending {
            if let Some(by) = newest {
                plan.close.push((older, Outcome::Superseded { by }));
            }
        }

        match (plan.running, newest) {
            (None, Some(newest)) => plan.running = Some((newest, Resume::Spawn)),
            (Some(_), newest) => plan.next = newest,
            (None, None) => {}
        }

        plan
    }
}

enum Verdict {
    Run(Resume),
    Close(Outcome),
}
