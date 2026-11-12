//! after the switch dropped ssh (sshd restarted, the stock user is gone) or
//! the network did: the runner carries on, and two ways back race. ssh
//! coming back follows the same job with `adopt --follow <offset>`; coeusd
//! answering on the tailnet follows it through an attach. there is no
//! deadline: after the guard's timeout the runner has kept the system or
//! rolled back, and one way or the other comes back to say which.

use std::{future, time::Duration};

use eyre::{Result, eyre};

use super::{Ending, Feed, FleetCoeus, POLL, Plan, backoff::Backoff, show, stock};
use crate::{
    bootstrap::{fleet::Daemon, remote::Stock, remote::Target},
    cli::Console,
    common::model::{JobSummary, StorePath, Timestamp},
    ui::checklist::Live,
};

/// what following the job again takes
pub struct Way<'a> {
    pub stock: &'a Stock,
    pub target: &'a Target,
    pub coeus: &'a FleetCoeus,
    pub source: &'a StorePath,
    pub plan: &'a Plan<'a>,
    pub job: &'a JobSummary,
}

/// the first way back to say anything about the job wins; the other is
/// dropped
pub async fn race(way: &Way<'_>, feed: &Feed, console: &Console) -> Result<(Ending, Feed)> {
    let host = way.plan.host;
    let waiting = Live::Waiting {
        since: Timestamp::now(),
        on: format!(
            "ssh dropped as the switch went on; waiting for {host} over ssh or the tailnet"
        ),
    };
    show(console, feed, Some(waiting));

    tokio::select! {
        result = by_ssh(way, resumed(feed), console) => result,
        result = by_tailnet(way, resumed(feed), console) => result,
        () = console.cancelled.cancelled() => Err(eyre!("detached while {host} switches")),
    }
}

/// how long ssh is left alone at most after adopt failed on it
const FAILING_AT_MOST: Duration = Duration::from_secs(30);

/// ssh back but adopt failing on it is as good as no ssh: the machine is
/// still switching (nix-daemon restarting under adopt's evaluation, sudo
/// gone with the stock user), so it is asked again, less often each time,
/// while the tailnet way may answer
async fn by_ssh(way: &Way<'_>, mut feed: Feed, console: &Console) -> Result<(Ending, Feed)> {
    let mut failing = Backoff::new(POLL, FAILING_AT_MOST);
    let mut wait = POLL;
    loop {
        tokio::time::sleep(wait).await;
        wait = POLL;
        if way.stock.ssh.connect(true).await.is_err() {
            continue;
        }
        let from = feed.offset;
        let adopted = stock::adopt(
            way.stock,
            way.coeus,
            way.source,
            way.plan,
            way.job.order,
            from,
            &mut feed,
            console,
        )
        .await;
        let ending = match adopted {
            Ok(ending) => ending,
            Err(error) => {
                wait = failing.next();
                let failed = Live::Tail(vec![format!(
                    "adopt failed over ssh, asking again in {}s: {error:#}",
                    wait.as_secs()
                )]);
                show(console, &feed, Some(failed));
                continue;
            }
        };
        failing.reset();
        match ending {
            Ending::Lost { .. } => continue,
            // the machine's own coeusd runs now; the tailnet way finds it
            Ending::DaemonHere => future::pending::<()>().await,
            Ending::Finished(_) => return Ok((ending, feed)),
        }
    }
}

async fn by_tailnet(way: &Way<'_>, mut feed: Feed, console: &Console) -> Result<(Ending, Feed)> {
    loop {
        tokio::time::sleep(POLL).await;
        let Ok(Some(daemon)) = Daemon::connect(way.target.settings(), &way.target.host).await
        else {
            continue;
        };
        let Ok(Some(job)) = daemon.job(way.job.order).await else {
            continue;
        };
        if let Ok(Some(done)) = daemon
            .follow(job.id, &mut feed, |feed| show(console, feed, None))
            .await
        {
            return Ok((Ending::Finished(done), feed));
        }
    }
}

/// a follower that goes on from where `feed` stopped
fn resumed(feed: &Feed) -> Feed {
    Feed {
        view: feed.view.clone(),
        offset: feed.offset,
        ..Feed::default()
    }
}
