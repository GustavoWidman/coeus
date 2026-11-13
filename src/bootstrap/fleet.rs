//! the machine as a member of the fleet: its coeusd answering over the
//! tailnet with the fleet key, its history, and the reboot that proves it
//! boots what it runs

use std::time::{Duration, Instant};

use eyre::{Result, bail, eyre};
use tokio::net::lookup_host;

use crate::{
    bootstrap::{
        Step,
        remote::{Call, Platform, Target},
        switch::{AdoptLine, Feed, Switched},
    },
    cli::{Console, within},
    client::{AttachmentEvent, CoeusClient, Fleet, Targets},
    common::model::{
        AttachFrame, HostInfo, Hostname, JobId, JobSelector, JobSummary, MachineView, OrderId,
        Timestamp,
    },
    config::ClientSettings,
    ui::{
        ask::{Answer, Prompt},
        checklist::{Live, Mark, Row},
    },
};

/// the machine's coeusd, as it introduced itself
pub struct Daemon {
    pub fleet: Fleet,
    pub hello: HostInfo,
    pub view: MachineView,
}

pub struct Answered {
    pub hello: HostInfo,
    pub view: MachineView,
    /// the address the name resolved to, for the summary
    pub address: String,
    pub fleet: Fleet,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reboot {
    /// it runs the kernel it booted
    NotNeeded,
    Rebooted {
        took: Duration,
    },
    /// the person chose not to now
    Declined,
}

/// how often a silent machine is asked again
const POLL: Duration = Duration::from_secs(3);

/// how far back history is searched for an order
const HISTORY: u32 = 64;

impl Daemon {
    /// coeusd answering as `host` within a few seconds, or `None`; a coeusd
    /// answering as another name is an error, not an absence
    pub async fn connect(settings: &ClientSettings, host: &Hostname) -> Result<Option<Self>> {
        let settings = &at_tailnet_address(settings, host).await;
        let key = settings.key.load()?;
        let targets = Targets::select(&[host.to_string()], settings)?;
        let fleet = Fleet::new(settings.clone(), key, targets);
        let Ok(mut client) = fleet.connect(host).await else {
            return Ok(None);
        };
        let hello = client.host().clone();
        if hello.hostname != *host {
            bail!(
                "{} answers as {}, not {host}",
                settings.endpoint(host).host,
                hello.hostname
            );
        }
        let Ok(view) = within(client.view()).await else {
            return Ok(None);
        };
        Ok(Some(Self { fleet, hello, view }))
    }

    pub async fn client(&self) -> Result<CoeusClient> {
        self.fleet.connect(&self.hello.hostname).await
    }

    /// the job of `order` in the machine's history
    pub async fn job(&self, order: OrderId) -> Result<Option<JobSummary>> {
        let history = within(self.client().await?.history(HISTORY)).await?;
        Ok(history.into_iter().find(|job| job.order == order))
    }

    /// the machine's recent jobs, newest first
    pub async fn history(&self) -> Result<Vec<JobSummary>> {
        within(self.client().await?.history(HISTORY)).await
    }

    /// follows `job`'s log from the feed's offset into the feed, calling
    /// `shown` after each change, until the job finishes (its summary) or
    /// coeusd hangs up (`None`)
    pub async fn follow(
        &self,
        job: JobId,
        feed: &mut Feed,
        mut shown: impl FnMut(&Feed),
    ) -> Result<Option<JobSummary>> {
        let client = self.client().await?;
        let mut attachment = client
            .attach(JobSelector::Id(job), feed.offset, None)
            .await?;
        loop {
            match attachment.next().await {
                Ok(Some(AttachmentEvent::Started(summary))) => {
                    feed.line(AdoptLine::Job(Box::new(summary)));
                }
                Ok(Some(AttachmentEvent::Frames { frames, end })) => {
                    for frame in frames {
                        attach_frame(feed, frame);
                    }
                    feed.line(AdoptLine::At(end));
                }
                Ok(Some(AttachmentEvent::Screen(_))) => {}
                Ok(Some(AttachmentEvent::Ended { .. })) => return Ok(feed.finished().cloned()),
                Ok(None) | Err(_) => return Ok(feed.finished().cloned()),
            }
            shown(feed);
        }
    }
}

fn attach_frame(feed: &mut Feed, frame: AttachFrame) {
    match frame {
        AttachFrame::Event(event) => feed.line(AdoptLine::Event(event)),
        AttachFrame::Nix(bytes) => {
            for line in String::from_utf8_lossy(&bytes).lines() {
                feed.line(AdoptLine::parse(line));
            }
        }
    }
}

/// coeusd answering as the host with the fleet key, running the switched
/// system. on a Mac that stays silent while ssh works, `tailscale status`
/// there says whether it is on the tailnet at all; when it is not, the row
/// says to sign in to Tailscale on it and keeps looking.
pub async fn answer(target: &Target, switched: &Switched, console: &Console) -> Result<Answered> {
    let host = &target.host;
    let since = Timestamp::now();
    loop {
        if console.cancelled.is_cancelled() {
            bail!("detached while waiting for {host}'s coeusd");
        }
        if let Some(daemon) = Daemon::connect(target.settings(), host).await? {
            let toplevel = switched.job.toplevel.as_ref();
            if let Some(toplevel) = toplevel
                && daemon.view.system.current.as_ref() != Some(toplevel)
            {
                console.note(format!(
                    "{host} answers, but runs {} rather than the system its switch built",
                    daemon
                        .view
                        .system
                        .current
                        .as_ref()
                        .map_or("nothing it can name", |current| current.name())
                ));
            }
            let endpoint = target.settings().endpoint(host);
            let address = lookup_host((endpoint.host.as_str(), endpoint.port))
                .await
                .ok()
                .and_then(|mut addresses| addresses.next())
                .map_or(endpoint.host.clone(), |address| address.ip().to_string());
            return Ok(Answered {
                hello: daemon.hello,
                view: daemon.view,
                address,
                fleet: daemon.fleet,
            });
        }

        let (mark, on) = match off_tailnet(target).await {
            true => (
                Mark::Blocked,
                format!("{host} is not on the tailnet: open Tailscale on it and sign in"),
            ),
            false => (Mark::Active, format!("{host}'s coeusd on the tailnet")),
        };
        console.update(Row {
            step: Step::Answer,
            mark,
            detail: format!("waiting for {host}'s coeusd"),
            live: vec![Live::Waiting { since, on }],
        });
        pause(console).await;
    }
}

/// whether a Mac reachable over ssh says it is not on the tailnet
async fn off_tailnet(target: &Target) -> bool {
    let Some(stock) = target.opened() else {
        return false;
    };
    if stock.probed.facts.platform != Platform::Darwin {
        return false;
    }
    let Ok(status) = stock.run(&Call::Tailscale).await else {
        return true;
    };
    serde_json::from_str::<serde_json::Value>(&status)
        .ok()
        .and_then(|status| status["BackendState"].as_str().map(str::to_owned))
        .is_none_or(|state| state != "Running")
}

/// when coeusd says the machine runs a kernel it did not boot, asks to
/// reboot now, then waits for coeusd to go and come back booted into it
pub async fn reboot(answered: &Answered, console: &Console) -> Result<Reboot> {
    let host = &answered.hello.hostname;
    if !answered.view.system.reboot_required {
        return Ok(Reboot::NotNeeded);
    }

    let question = format!("{host} runs a kernel it did not boot. reboot it now and wait for it?");
    match console.ask(Prompt::confirm(question, true)).await {
        Ok(Answer::Value(yes)) if yes == "true" => {}
        Ok(Answer::Cancelled) => return Err(eyre!("cancelled before rebooting {host}")),
        Ok(_) => return Ok(Reboot::Declined),
        Err(_) => {
            console.note(format!(
                "{host} runs a kernel it did not boot; `coeus reboot {host}` boots it"
            ));
            return Ok(Reboot::Declined);
        }
    }

    within(answered.fleet.connect(host).await?.reboot()).await?;
    let started = Instant::now();
    let since = Timestamp::now();
    loop {
        console.update(Row {
            step: Step::Reboot,
            mark: Mark::Active,
            detail: format!("rebooting {host}"),
            live: vec![Live::Waiting {
                since,
                on: format!("{host} to boot and its coeusd to answer"),
            }],
        });
        pause(console).await;
        if console.cancelled.is_cancelled() {
            bail!("detached while {host} reboots");
        }
        let Ok(mut client) = answered.fleet.connect(host).await else {
            continue;
        };
        if let Ok(view) = within(client.view()).await
            && !view.system.reboot_required
        {
            return Ok(Reboot::Rebooted {
                took: started.elapsed(),
            });
        }
    }
}

async fn pause(console: &Console) {
    tokio::select! {
        () = tokio::time::sleep(POLL) => {}
        () = console.cancelled.cancelled() => {}
    }
}

/// `settings` reaching `host` at its tailnet address when the inventory names
/// none. a machine that just joined is in the local tailscale's peer list at
/// once, while the system resolver may still remember that its name did not
/// exist moments ago (macOS caches that answer).
async fn at_tailnet_address(settings: &ClientSettings, host: &Hostname) -> ClientSettings {
    let mut settings = settings.clone();
    if settings
        .hosts
        .get(host)
        .is_some_and(|entry| entry.address.is_some())
    {
        return settings;
    }
    for program in [
        "tailscale",
        "/Applications/Tailscale.app/Contents/MacOS/Tailscale",
    ] {
        let status = tokio::process::Command::new(program)
            .args(["status", "--json"])
            .kill_on_drop(true)
            .output();
        let Ok(Ok(output)) = tokio::time::timeout(Duration::from_secs(5), status).await else {
            continue;
        };
        if let Some(address) = peer_address(&output.stdout, host.as_str()) {
            settings.hosts.entry(host.clone()).or_default().address = Some(address);
        }
        break;
    }
    settings
}

/// the first tailnet address of the peer MagicDNS names `host`, from
/// `tailscale status --json`
pub(crate) fn peer_address(status: &[u8], host: &str) -> Option<String> {
    let status: serde_json::Value = serde_json::from_slice(status).ok()?;
    let named = |peer: &&serde_json::Value| {
        peer["DNSName"]
            .as_str()
            .and_then(|name| name.split('.').next())
            .is_some_and(|label| label.eq_ignore_ascii_case(host))
    };
    let peer = status["Peer"].as_object()?.values().find(named)?;
    peer["TailscaleIPs"]
        .as_array()?
        .first()?
        .as_str()
        .map(str::to_owned)
}
