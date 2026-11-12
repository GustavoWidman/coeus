use clap::Args;
use eyre::{Result, bail};

use crate::{
    cli::{Context, exit::Exit},
    common::model::{CancelReport, JobId, JobSelector},
    ui::{Theme, board::Verdict, say},
};

#[derive(Args, Debug)]
pub struct CancelArgs {
    /// Machines whose jobs to cancel
    #[arg(value_name = "TARGETS", required = true)]
    pub targets: Vec<String>,

    /// Cancel this job instead of the current one (one machine only)
    #[arg(long, value_name = "N")]
    pub job: Option<JobId>,
}

impl CancelArgs {
    pub async fn run(self, context: &Context) -> Result<Exit> {
        let targets = context.targets(&self.targets)?;
        if self.job.is_some() && targets.len() > 1 {
            bail!("job numbers belong to one machine; --job takes a single target");
        }

        let selector = self.job.map_or(JobSelector::Current, JobSelector::Id);
        let fleet = context.fleet(targets)?;
        let reports = fleet
            .each(|_, mut client| async move { client.cancel(selector).await })
            .await;

        let mut verdicts = Vec::new();
        for (host, report) in reports {
            let host = Theme::host(host.as_str());
            let line = match report {
                Ok(CancelReport::Withdrawn(job)) => {
                    say::success(format!("{host} withdrew the queued job {job}"))
                }
                Ok(CancelReport::Stopping(job)) => {
                    say::success(format!("{host} is stopping job {job}"))
                }
                Ok(CancelReport::TooLate(job)) => say::warn(format!(
                    "{host} is activating job {job}, which has to finish"
                )),
                Ok(CancelReport::Nothing) => say::aside(format!("{host} has nothing to cancel")),
                Err(error) => {
                    verdicts.push(Verdict::Unreachable);
                    say::error(format!("{host} is unreachable: {error}"))
                }
            };
            println!("{line}");
        }

        Ok(Exit::of(verdicts, false))
    }
}
