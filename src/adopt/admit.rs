//! finding or admitting the job of one bootstrap order, and taking up one
//! left unfinished the way coeusd does at startup

use eyre::{Result, bail};

use crate::{
    common::model::{
        ATTEMPTS, Attempt, JobId, JobSummary, NarHash, OrderId, Outcome, StorePath, bootstrap_order,
    },
    engine::reconcile::{Observed, Plan, Resume},
    job::{JobDir, JobSpec, JobStore},
};

#[derive(Debug)]
pub enum Admitted {
    /// the order's job, admitted now: its runner is to be started
    Created(JobDir),
    /// the order's job, admitted by an earlier run
    Found(JobDir),
}

/// the job of `order`, one of `nar_hash`'s bootstrap orders. a new job is
/// admitted only when `order` is the snapshot's next try and nothing else
/// runs, so the same order never makes two jobs.
pub async fn admit(
    store: &JobStore,
    nar_hash: &NarHash,
    order: OrderId,
    spec: impl FnOnce(JobId) -> JobSpec,
) -> Result<Admitted> {
    if !(0..ATTEMPTS).any(|attempt| bootstrap_order(nar_hash, attempt) == order) {
        bail!("order {order} is not a bootstrap order of {nar_hash}");
    }

    if let Some(dir) = store.find(order)? {
        return Ok(Admitted::Found(dir));
    }

    let jobs = summaries(store).await?;
    let refusal = match Attempt::of(&jobs, nar_hash) {
        Attempt::Fresh(next)
        | Attempt::Failed {
            next: Some(next), ..
        } if next == order => None,
        Attempt::Fresh(next)
        | Attempt::Failed {
            next: Some(next), ..
        } => Some(format!(
            "order {order} is not the next try of {nar_hash}, {next} is"
        )),
        Attempt::Running(job) => Some(format!("job {} already switches to {nar_hash}", job.id)),
        Attempt::Succeeded(job) => Some(format!("job {} already switched to {nar_hash}", job.id)),
        Attempt::Failed { next: None, .. } => Some(format!(
            "all {ATTEMPTS} tries of switching to {nar_hash} failed"
        )),
    };
    if let Some(refusal) = refusal {
        bail!(refusal);
    }

    if let Some(job) = jobs.iter().find(|job| !job.state.is_finished()) {
        bail!("job {} has not finished yet", job.id);
    }

    let dir = store.create(&spec(store.next_id()?))?;
    Ok(Admitted::Created(dir))
}

/// every job on disk as it stands, oldest first
pub async fn summaries(store: &JobStore) -> Result<Vec<JobSummary>> {
    let mut summaries = Vec::new();
    for id in store.list()? {
        summaries.push(store.dir(id).view().await?.summary);
    }
    Ok(summaries)
}

/// what became of a job found on disk
#[derive(Debug, PartialEq, Eq)]
pub enum Revival {
    /// it finished before
    Finished,
    /// a live runner holds it
    Running,
    /// no runner holds it and it may run again: start one
    Spawn,
    /// no runner holds it and it may not run again: closed with this
    Closed(Outcome),
}

/// does to an unfinished job no coeusd follows what coeusd does at startup
/// (see [Plan::reconcile]); `current` is the system the machine runs
pub async fn revive(dir: &JobDir, current: Option<&StorePath>) -> Result<Revival> {
    let view = dir.view().await?;
    if view.summary.state.is_finished() {
        return Ok(Revival::Finished);
    }

    let observed = Observed {
        id: dir.id(),
        state: view.summary.state,
        held: dir.log().is_held()?,
        stop: dir.stop_request()?,
        toplevel: view.summary.toplevel,
    };

    let mut plan = Plan::reconcile(vec![observed], current);
    if let Some((_, resume)) = plan.running {
        return Ok(match resume {
            Resume::Follow => Revival::Running,
            Resume::Spawn => Revival::Spawn,
        });
    }

    let Some((_, outcome)) = plan.close.pop() else {
        bail!("job {} is unfinished, yet nothing can take it up", dir.id());
    };
    dir.close(outcome.clone())?;
    Ok(Revival::Closed(outcome))
}
