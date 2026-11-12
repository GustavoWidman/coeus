//! the switch through a coeusd that answers: the job of this release's
//! orders read from history, followed, or ordered like `deploy --local`

use eyre::{Result, bail, eyre};

use super::{Door, Feed, Plan, Switched, finished, show};
use crate::{
    bootstrap::{Blocker, fleet::Daemon},
    cli::{Console, within},
    client::{Snapshot, detect_origin},
    common::model::{Action, Attempt, JobId, NixOptions, Order, OrderId},
};

pub async fn switch(daemon: &Daemon, plan: &Plan<'_>, console: &Console) -> Result<Switched> {
    let history = daemon.history().await?;
    let job = match Attempt::of(&history, &plan.release.nar_hash) {
        Attempt::Succeeded(job) => {
            return Ok(Switched {
                job,
                via: Door::Ordered,
            });
        }
        Attempt::Running(job) => job.id,
        Attempt::Fresh(order) => place(daemon, plan, order).await?,
        Attempt::Failed { last, next } => match (plan.retry, next) {
            (true, Some(next)) => place(daemon, plan, next).await?,
            (true, None) => {
                return Err(eyre!(
                    "every try of this head failed on {}; commit a fix in the clone and run again",
                    plan.host
                ));
            }
            (false, _) => {
                return Err(Blocker::SwitchFailed {
                    job: Box::new(last),
                }
                .into());
            }
        },
    };
    follow(daemon, job, console).await
}

/// the clone's head uploaded and ordered; a machine answers a repeated
/// order with the job it already made
async fn place(daemon: &Daemon, plan: &Plan<'_>, order: OrderId) -> Result<JobId> {
    let snapshot = Snapshot::capture(&plan.clone).await?;
    if snapshot.nar_hash != plan.release.nar_hash {
        bail!(
            "the clone holds {}, not the {} the switch is for: run again",
            snapshot.nar_hash,
            plan.release.nar_hash
        );
    }

    let mut client = daemon.client().await?;
    client.upload(&snapshot).await?;
    let order = Order {
        id: order,
        source: plan.release.source(),
        action: Action::Switch,
        options: NixOptions::default(),
        origin: detect_origin(),
        now: false,
    };
    match within(client.deploy(order)).await? {
        Ok(admission) => Ok(admission.job),
        Err(rejection) => Err(eyre!("{} refused the switch: {rejection:?}", plan.host)),
    }
}

/// the job's log through coeusd until it finishes, reconnecting when coeusd
/// hangs up, as it does when it restarts into the system it switched to
async fn follow(daemon: &Daemon, job: JobId, console: &Console) -> Result<Switched> {
    let mut feed = Feed::default();
    loop {
        if let Some(job) = daemon
            .follow(job, &mut feed, |feed| show(console, feed, None))
            .await?
        {
            return finished(job, Door::Ordered, &feed, console);
        }
        if console.cancelled.is_cancelled() {
            bail!("detached from the switch");
        }
        tokio::time::sleep(super::POLL).await;
    }
}
