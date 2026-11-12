//! the first switch over ssh: ship, build the fleet's coeus, adopt

use super::{Feed, Release};
use crate::{
    bootstrap::remote::Stock,
    cli::Console,
    common::model::{Attempt, JobSummary, StorePath},
};

/// how a stream of `coeusd adopt --follow` ended
pub enum Ending {
    Finished(JobSummary),
    /// the stream ended without the job finishing, at this log offset
    Lost {
        job: JobSummary,
        offset: u64,
    },
    /// adopt exited 3: a coeusd serves on the machine, order through it
    DaemonHere,
}

/// the coeus the machine's configuration pins, built on the machine
pub struct FleetCoeus {
    pub path: StorePath,
    pub version: String,
}

pub async fn switch(
    stock: &Stock,
    release: &Release,
    retry: bool,
    console: &Console,
) -> eyre::Result<Ending> {
    // TODO: ship → fleet_coeus → status → adopt the attempt's order, from 0
    todo!()
}

/// `nix flake archive --json --to ssh-ng://<dest>` of the clone through the
/// pinned master: the source and every locked input land in the machine's
/// store, so it evaluates exactly what this admin does without any of the
/// fleet's credentials. a rerun copies only what is missing. returns the
/// source's store path there.
pub async fn ship(stock: &Stock, release: &Release, console: &Console) -> eyre::Result<StorePath> {
    todo!()
}

/// `<configurations>.<host>.config.services.coeus.package` built as root,
/// the first evaluation of the machine's configuration, so its errors are
/// the person's to read. a coeus without `adopt` is refused naming the
/// input to update.
pub async fn fleet_coeus(
    stock: &Stock,
    source: &StorePath,
    release: &Release,
    console: &Console,
) -> eyre::Result<FleetCoeus> {
    todo!()
}

/// `coeusd adopt --status`, where this snapshot's switch stands on the machine
pub async fn status(stock: &Stock, coeus: &FleetCoeus, release: &Release) -> eyre::Result<Attempt> {
    todo!()
}

/// runs `coeusd adopt … --follow <from>` as root and feeds its lines to the
/// switch row until the job finishes or the stream ends
pub async fn adopt(
    stock: &Stock,
    coeus: &FleetCoeus,
    source: &StorePath,
    release: &Release,
    order: crate::common::model::OrderId,
    from: u64,
    feed: &mut Feed,
) -> eyre::Result<Ending> {
    todo!()
}
