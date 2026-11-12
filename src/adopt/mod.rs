//! `coeusd adopt`: admits a job on a machine no coeusd runs on yet, the way
//! coeusd admits one, so the coeusd the switch installs finds it on disk,
//! follows its runner and keeps it in its history. it runs from the coeus
//! the machine's configuration pins, so the job directory is written by the
//! same code that will read it.
//!
//! it is idempotent: the order id comes from the snapshot, so running it
//! again finds the job, and does to an unfinished one what coeusd does at
//! startup (follow it, start a runner again, or close it by what the
//! machine runs).

// the adopt unit fills this module in
#![allow(dead_code, unused_variables)]

mod follow;
mod settings;

use crate::{
    cli::daemon::AdoptArgs,
    common::model::{JobId, OrderId},
    job::JobSpec,
};

/// what adopt exits with when a coeusd already serves here: the CLI orders
/// through it instead
pub const DAEMON_HERE: i32 = 3;

/// the process's exit code
pub async fn run(args: &AdoptArgs) -> eyre::Result<i32> {
    // TODO:
    // if args.status { print Attempt::of(store's summaries, nar_hash) as JSON; return 0 }
    // if settings::listening(port).await { return Ok(DAEMON_HERE) }   // one owner of the job store
    // let settings = Settings::of(source, hostname).await?;
    // Snapshots::new(sources).place(source, nar_hash)?;
    // order must be one of bootstrap_order(nar_hash, 0..ATTEMPTS), else refuse
    // match the store's job for `order`:
    //     none → JobStore::create(spec(…)) (atomic), start its runner through
    //            the shared Launcher
    //     unfinished with no live runner → reconcile it like coeusd's startup
    //     otherwise → leave it
    // if let Some(from) = args.follow { follow::stream(&dir, from).await? }
    todo!()
}

/// the job coeusd would have written for this order
fn spec(args: &AdoptArgs, order: OrderId, settings: &settings::Settings, id: JobId) -> JobSpec {
    // TODO: Order { id: order, source: Source::Snapshot { nar_hash, label },
    // action: Switch, options: default, origin: User { the sudo user, label's
    // admin }, now: false }; target and guard from settings;
    // install_bootloader from args
    todo!()
}
