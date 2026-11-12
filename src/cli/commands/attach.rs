use std::time::Duration;

use clap::Args;
use eyre::Result;

use crate::{
    cli::{
        Context,
        attach::{Attach, Mode},
        exit::Exit,
    },
    common::model::{Hostname, JobId, JobSelector},
};

#[derive(Args, Debug)]
pub struct AttachArgs {
    pub host: Hostname,

    /// The job to follow [default: the running one, else the queued one,
    /// else the latest]
    pub job: Option<JobId>,

    /// Write nix's internal-json lines to stdout instead, for other tools
    #[arg(long)]
    pub raw: bool,
}

#[derive(Args, Debug)]
pub struct LogsArgs {
    pub host: Hostname,

    /// The job to print [default: the running one, else the queued one,
    /// else the latest]
    pub job: Option<JobId>,

    /// Keep printing until the job finishes
    #[arg(short, long)]
    pub follow: bool,
}

impl AttachArgs {
    pub async fn run(self, context: &Context) -> Result<Exit> {
        let mode = match (self.raw, context.live) {
            (true, _) => Mode::Raw,
            (false, true) => Mode::Live,
            (false, false) => Mode::Transcript,
        };
        follow(context, &self.host, self.job, mode, None).await
    }
}

impl LogsArgs {
    /// how long a log without `--follow` may stay quiet before it counts as
    /// read to its current end
    const CAUGHT_UP: Duration = Duration::from_millis(800);

    pub async fn run(self, context: &Context) -> Result<Exit> {
        let idle = (!self.follow).then_some(Self::CAUGHT_UP);
        follow(context, &self.host, self.job, Mode::Transcript, idle).await
    }
}

async fn follow(
    context: &Context,
    host: &Hostname,
    job: Option<JobId>,
    mode: Mode,
    idle: Option<Duration>,
) -> Result<Exit> {
    let fleet = context.machine(host)?;
    let attached = Attach {
        fleet: &fleet,
        host,
        job: job.map_or(JobSelector::Current, JobSelector::Id),
        mode,
        idle,
        diff_limit: Some(12),
    }
    .run()
    .await?;

    Ok(match attached.and_then(|attached| attached.outcome) {
        Some(outcome) if outcome.is_success() => Exit::Success,
        Some(_) => Exit::Failed,
        None if idle.is_some() => Exit::Success,
        None => Exit::Detached,
    })
}
