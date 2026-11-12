//! the machine as a member of the fleet: its coeusd answering over the
//! tailnet with the fleet key, its history, and the reboot that proves it
//! boots what it runs

use std::time::Duration;

use crate::{
    bootstrap::{remote::Target, switch::Switched},
    cli::Console,
    client::Fleet,
    common::model::{HostInfo, Hostname, JobSummary, MachineView, OrderId},
    config::ClientSettings,
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

impl Daemon {
    /// coeusd answering as `host` within a few seconds, or `None`; a coeusd
    /// answering as another name is an error, not an absence
    pub async fn connect(settings: &ClientSettings, host: &Hostname) -> eyre::Result<Option<Self>> {
        todo!()
    }

    /// the job of `order` in the machine's history
    pub async fn job(&self, order: OrderId) -> eyre::Result<Option<JobSummary>> {
        todo!()
    }
}

/// coeusd answering as the host with the fleet key, running the switched
/// system. on a Mac that stays silent while ssh works, `tailscale status`
/// there says whether it is on the tailnet at all; when it is not, the row
/// says to sign in to Tailscale on it and keeps looking.
pub async fn answer(
    target: &Target,
    switched: &Switched,
    console: &Console,
) -> eyre::Result<Answered> {
    todo!()
}

/// when coeusd says the machine runs a kernel it did not boot, asks to
/// reboot now, then waits for coeusd to go and come back booted into it
pub async fn reboot(answered: &Answered, console: &Console) -> eyre::Result<Reboot> {
    todo!()
}
