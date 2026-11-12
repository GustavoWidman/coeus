//! the machine being bootstrapped, through whichever door is open: ssh
//! while it runs its stock system, coeusd over the tailnet once it runs the
//! fleet's. after the first switch the stock user may be gone, so a run
//! where coeusd answers never opens ssh unless a step asks for it.

mod call;
mod facts;
mod nix;
mod probe;
mod session;
mod ssh;
mod sudo;

#[cfg(test)]
pub(crate) mod test;

use tokio::sync::OnceCell;

pub use call::{AdoptCall, Call};
pub use facts::{BootLoader, Facts, Platform, System};
pub use nix::{NixState, converge as converge_nix, move_etc_aside};
pub use probe::Probed;
pub use session::SessionKey;
pub use ssh::Ssh;
pub use sudo::Privilege;

use crate::{
    bootstrap::{Bootstrap, SshKey, fleet::Daemon},
    cli::Console,
    common::model::Hostname,
};

pub struct Target {
    pub host: Hostname,
    /// coeusd answered as `host` with the fleet key
    pub daemon: Option<Daemon>,
    /// ssh, opened on first use
    stock: OnceCell<Stock>,
    door: Door,
}

/// the machine as its stock system shows it over ssh, as root through the
/// session key once it is installed
pub struct Stock {
    pub ssh: Ssh,
    pub privilege: Privilege,
    pub probed: Probed,
}

/// how to open ssh, once needed
struct Door {
    destination: Option<String>,
    pin: Option<SshKey>,
}

/// tries coeusd and ssh at once. coeusd answering is enough; otherwise ssh
/// must, and a first contact asks the person to trust the key it presents.
pub async fn reach(bootstrap: &Bootstrap, console: &Console) -> eyre::Result<Target> {
    // TODO:
    // let daemon = Daemon::connect(&settings, host)        // a few seconds; hello.hostname == host
    // match (daemon, saved.destination) {
    //     (Some(d), _) => Target { daemon: Some(d), stock: lazy }
    //     (None, None) => bail!("say how to reach {host} with --to user@address")
    //     (None, Some(dest)) => {
    //         let pin = Ssh::pinned(state)?.or(first contact, confirmed by the person
    //                   or by options.host_key)
    //         open(): session key works → root at once; else login (ssh may
    //         prompt: console.hand_over), Privilege::establish, SessionKey::install
    //         probe; require probed.host_key == pin; Facts::platform is
    //         NixOS or darwin (adopt only)
    //     }
    // }
    todo!()
}

impl Target {
    /// the stock machine over ssh, opened now if it was not
    pub async fn stock(&self, bootstrap: &Bootstrap, console: &Console) -> eyre::Result<&Stock> {
        todo!()
    }

    /// what the reach row says: the machine, or the coeusd that answered
    pub fn describe(&self) -> String {
        todo!()
    }
}
