use clap::Args;
use eyre::Result;
use serde_json::json;

use crate::{
    cli::{Context, exit::Exit},
    common::model::{Hostname, JobSummary, Timestamp},
    ui::{board::Verdict, history, say},
};

#[derive(Args, Debug)]
pub struct HistoryArgs {
    /// Machines whose jobs to list [default: @all]
    #[arg(value_name = "TARGETS")]
    pub targets: Vec<String>,

    /// How many jobs to show
    #[arg(short = 'n', long, value_name = "N", default_value_t = 20)]
    pub limit: u32,

    /// Print the jobs as JSON
    #[arg(long)]
    pub json: bool,
}

impl HistoryArgs {
    pub async fn run(self, context: &Context) -> Result<Exit> {
        let fleet = context.fleet(context.targets_or_all(&self.targets)?)?;
        let limit = self.limit;
        let answers = fleet
            .each(|_, mut client| async move { client.history(limit).await })
            .await;

        let mut jobs: Vec<(Hostname, JobSummary)> = Vec::new();
        let mut unreachable = Vec::new();
        for (host, answer) in answers {
            match answer {
                Ok(summaries) => jobs.extend(summaries.into_iter().map(|job| (host.clone(), job))),
                Err(error) => {
                    eprintln!("{}", say::warn(format!("{host} is unreachable: {error}")));
                    unreachable.push(Verdict::Unreachable);
                }
            }
        }
        jobs.sort_by_key(|(_, job)| std::cmp::Reverse(job.ordered_at));
        jobs.truncate(limit as usize);

        if self.json {
            let jobs: Vec<_> = jobs
                .iter()
                .map(|(host, job)| json!({ "host": host, "job": job }))
                .collect();
            println!("{}", serde_json::to_string_pretty(&jobs)?);
        } else if jobs.is_empty() {
            println!("{}", say::aside("no jobs yet"));
        } else {
            for line in history::jobs(&jobs, Timestamp::now()) {
                println!("{line}");
            }
        }

        Ok(Exit::of(unreachable, false))
    }
}
