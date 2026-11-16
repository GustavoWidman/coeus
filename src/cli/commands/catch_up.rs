use std::time::Duration;

use clap::Args;
use colored::Colorize;
use eyre::{Result, eyre};

use crate::{
    cli::{Context, exit::Exit},
    common::model::{Hostname, Pinned},
    ui::{Theme, board::Verdict, say},
};

/// how long a machine may take to answer: its lookup gives up after two
/// minutes, and may first wait for one already under way
const ANSWER: Duration = Duration::from_secs(4 * 60);

#[derive(Args, Debug)]
pub struct CatchUpArgs {
    /// Machines that should look up the ref they follow
    #[arg(value_name = "TARGETS", required = true)]
    pub targets: Vec<String>,

    /// Exit 0 when the only problem is machines that could not be reached
    #[arg(long)]
    pub skip_unreachable: bool,
}

impl CatchUpArgs {
    pub async fn run(self, context: &Context) -> Result<Exit> {
        let fleet = context.fleet(context.targets(&self.targets)?)?;
        let answers = fleet
            .each(|host, mut client| async move {
                let answer = tokio::time::timeout(ANSWER, client.catch_up())
                    .await
                    .unwrap_or_else(|_| {
                        Err(eyre!("{host}: no answer within {}s", ANSWER.as_secs()))
                    });
                Ok(answer)
            })
            .await;

        let mut verdicts = Vec::new();
        for (host, answer) in answers {
            let (verdict, line) = match answer {
                Ok(Ok(pinned)) => (Verdict::Done, sees(&host, &pinned)),
                Ok(Err(error)) => (Verdict::Rejected, say::error(error)),
                Err(error) => (
                    Verdict::Unreachable,
                    say::error(format!(
                        "{} is unreachable: {error}",
                        Theme::host(host.as_str())
                    )),
                ),
            };
            verdicts.push(verdict);
            println!("{line}");
        }

        Ok(Exit::of(verdicts, self.skip_unreachable))
    }
}

/// `[✓] oracle-1 sees deployed at c3e959b`
fn sees(host: &Hostname, pinned: &Pinned) -> String {
    say::success(format!(
        "{} sees {} at {}",
        Theme::host(host.as_str()),
        pinned.git_ref.bold(),
        pinned.rev.short().cyan()
    ))
}
