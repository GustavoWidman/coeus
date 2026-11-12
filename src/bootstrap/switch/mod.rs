//! the machine runs the branch's head. while it runs its stock system: the
//! clone and its inputs are shipped into its store, the coeus its own
//! configuration pins is built there, and that coeus's `coeusd adopt` admits
//! the job and starts its runner the way coeusd would. once its coeusd
//! answers: the job's outcome is read from history, and a head that moved
//! since is ordered through the protocol like `deploy --local`. either way
//! the switch is an ordinary job, built, activated and guarded on the
//! machine whatever happens to the connection or to the admin.

mod feed;
mod stock;

pub use feed::{AdoptLine, Feed};
pub use stock::{Ending, FleetCoeus};

use crate::{
    bootstrap::{Bootstrap, remote::Target},
    cli::Console,
    common::model::{CommitSha, JobSummary, NarHash, Source},
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

pub async fn converge(
    release: &Release,
    target: &Target,
    bootstrap: &Bootstrap,
    console: &Console,
) -> eyre::Result<Switched> {
    // TODO:
    // loop {
    //   if coeusd answers now: Attempt::of(history, nar_hash) → follow,
    //       or upload + order the attempt's order like `deploy --local`
    //   else: stock::switch over ssh:
    //       Finished(job) → done (Failed without --retry → Blocker::SwitchFailed)
    //       DaemonHere → loop (coeusd took the machine over)
    //       Lost { offset } → the switch dropped ssh; race ssh coming back
    //           (`adopt --follow <offset>`) against coeusd answering (attach),
    //           without a deadline; ctrl-c detaches
    // }
    todo!()
}
