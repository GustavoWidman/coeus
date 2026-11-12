//! the machine runs the branch's head. while it runs its stock system: the
//! clone and its inputs are shipped into its store, the coeus its own
//! configuration pins is built there, and that coeus's `coeusd adopt` admits
//! the job and starts its runner the way coeusd would. once its coeusd
//! answers: the job's outcome is read from history, and a head that moved
//! since is ordered through the protocol like `deploy --local`. either way
//! the switch is an ordinary job, built, activated and guarded on the
//! machine whatever happens to the connection or to the admin.

mod daemon;
mod feed;
mod rejoin;
mod ship;
mod stock;

#[cfg(test)]
mod test;

use std::time::Duration;

use eyre::{Result, bail};

pub use feed::{AdoptLine, Feed};
pub use stock::{Ending, FleetCoeus, Plan};

use crate::{
    bootstrap::{Blocker, Bootstrap, Step, fleet::Daemon, remote::Target},
    cli::Console,
    common::model::{CommitSha, JobState, JobSummary, NarHash, Outcome, Source},
    ui::checklist::{Live, Mark, Row},
};

/// what the machine should run: the clone's committed head as a snapshot
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
    pub head: CommitSha,
    pub nar_hash: NarHash,
    /// `<admin>:coeus/bootstrap/<host>@<short>`
    pub label: String,
    pub install_bootloader: bool,
}

impl Release {
    pub fn source(&self) -> Source {
        Source::Snapshot {
            nar_hash: self.nar_hash.clone(),
            label: self.label.clone(),
        }
    }
}

/// the job that put the machine on the release
#[derive(Clone, Debug)]
pub struct Switched {
    pub job: JobSummary,
    pub via: Door,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Door {
    /// `coeusd adopt` over ssh
    Adopted,
    /// an order through coeusd
    Ordered,
}

/// how long a machine that has a coeusd starting up is given before it is
/// asked again
const POLL: Duration = Duration::from_secs(3);

pub async fn converge(
    release: &Release,
    target: &Target,
    bootstrap: &Bootstrap,
    console: &Console,
) -> Result<Switched> {
    let plan = Plan::of(release, bootstrap);
    loop {
        if console.cancelled.is_cancelled() {
            bail!("detached from the switch");
        }
        if let Some(daemon) = Daemon::connect(target.settings(), &target.host).await? {
            return daemon::switch(&daemon, &plan, console).await;
        }

        let stock = target.stock(bootstrap, console).await?;
        let mut feed = Feed::default();
        let (coeus, source, ending) = stock::switch(stock, &plan, &mut feed, console).await?;
        let mut ending = ending;
        loop {
            match ending {
                Ending::Finished(job) => return finished(job, Door::Adopted, &feed, console),
                // a coeusd took the machine over: order through it
                Ending::DaemonHere => {
                    tokio::time::sleep(POLL).await;
                    break;
                }
                Ending::Lost { job, .. } => {
                    let way = rejoin::Way {
                        stock,
                        target,
                        coeus: &coeus,
                        source: &source,
                        plan: &plan,
                        job: &job,
                    };
                    (ending, feed) = rejoin::race(&way, &feed, console).await?;
                }
            }
        }
    }
}

/// the switch done, or a blocker saying why not, with nix's last words as
/// notes
fn finished(job: JobSummary, via: Door, feed: &Feed, console: &Console) -> Result<Switched> {
    match &job.state {
        JobState::Finished(Outcome::Succeeded | Outcome::UpToDate) => Ok(Switched { job, via }),
        _ => {
            for line in feed.failure() {
                console.note(line);
            }
            Err(Blocker::SwitchFailed { job: Box::new(job) }.into())
        }
    }
}

/// the switch row: the job, drawn with the deploy board's cells
fn show(console: &Console, feed: &Feed, waiting: Option<Live>) {
    let Some(view) = &feed.view else { return };
    let summary = &view.summary;
    let detail = match &summary.state {
        JobState::Queued => format!("job {} queued", summary.id),
        JobState::Running { phase } => format!("job {} {phase}", summary.id),
        JobState::Finished(outcome) => format!("job {} {outcome}", summary.id),
    };
    let mut live = vec![Live::Job(Box::new(view.clone()))];
    live.extend(waiting);
    console.update(Row {
        step: Step::Switch,
        mark: Mark::Active,
        detail,
        live,
    });
}
