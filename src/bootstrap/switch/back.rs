//! the machine coming back from the reboot into its boot job's system: its
//! coeusd answering on the tailnet, or ssh finding it runs that system.
//! there is no deadline, as for the switch itself; ssh finding it booted
//! into anything else ends the wait.

use eyre::{Result, bail};

use super::{POLL, first::Back};
use crate::{
    bootstrap::{
        Step,
        fleet::Daemon,
        remote::{Call, Running, Stock, Target},
    },
    cli::Console,
    common::model::{JobSummary, Timestamp},
    ui::checklist::{Live, Mark, Row},
};

pub async fn wait(
    target: &Target,
    stock: &Stock,
    job: &JobSummary,
    before: &Running,
    console: &Console,
) -> Result<()> {
    let host = &target.host;
    let since = Timestamp::now();
    loop {
        console.update(Row {
            step: Step::Switch,
            mark: Mark::Active,
            detail: format!("rebooting {host} into its fleet system"),
            live: vec![Live::Waiting {
                since,
                on: format!("{host} to boot its fleet system"),
            }],
        });
        tokio::select! {
            () = tokio::time::sleep(POLL) => {}
            () = console.cancelled.cancelled() => {
                bail!("detached while {host} reboots");
            }
        }

        if Daemon::connect(target.settings(), host).await?.is_some() {
            return Ok(());
        }
        if stock.ssh.connect(true).await.is_err() {
            continue;
        }
        let Ok(now) = stock.run(&Call::Running).await else {
            continue;
        };
        let Ok(now) = Running::parse(&now) else {
            continue;
        };
        match Back::of(job, before, &now) {
            Back::NotYet => {}
            Back::Home => return Ok(()),
            Back::Elsewhere(system) => {
                let system = system
                    .as_ref()
                    .map_or("a system it cannot name", |system| system.name());
                bail!(
                    "{host} rebooted into {system} rather than the system its boot job made the default"
                );
            }
        }
    }
}
