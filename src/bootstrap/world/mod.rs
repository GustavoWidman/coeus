//! what a run reaches outside the admin: the machine and the forge. a run
//! talks to them only through these two traits, so the steps' order and
//! their convergence are tested against fakes; [Live] and [Gh] are the real
//! ones, over ssh, coeusd and `gh`.

mod gh;
mod live;

use std::path::Path;

use eyre::Result;

pub use gh::Gh;
pub use live::Live;

use super::{
    Bootstrap,
    branch::PullRequest,
    fleet::Reboot,
    remote::Probed,
    switch::{Release, Switched},
};
use crate::{
    cli::Console,
    common::model::{FdaStatus, Platform},
};

/// the machine being bootstrapped
pub(crate) trait Machine {
    /// its coeusd answering, else ssh to its stock system: the door every
    /// later step goes through
    async fn reach(&self, bootstrap: &Bootstrap, console: &Console) -> Result<Reached>;

    /// the stock system as the probe found it, over ssh opened now when it
    /// was not
    async fn probed(&self, bootstrap: &Bootstrap, console: &Console) -> Result<Probed>;

    /// Determinate Nix on a Mac, and the files nix-darwin would refuse to
    /// replace moved aside
    async fn nix(&self, bootstrap: &Bootstrap, console: &Console) -> Result<NixReady>;

    /// the machine on `release`, through `coeusd adopt` or coeusd
    async fn switch(
        &self,
        release: &Release,
        bootstrap: &Bootstrap,
        console: &Console,
    ) -> Result<Switched>;

    /// its coeusd answering over the tailnet with the fleet key
    async fn answer(&self, switched: &Switched, console: &Console) -> Result<Answer>;

    /// a reboot when it runs a kernel it did not boot, asked first
    async fn reboot(&self, answer: &Answer, console: &Console) -> Result<Reboot>;

    /// the session key out of root's `authorized_keys` and out of the state
    /// directory; whether there was one to remove
    async fn cleanup(&self, bootstrap: &Bootstrap, console: &Console) -> Result<bool>;
}

/// the pull request of the enrollment branch
pub(crate) trait Forge {
    /// the newest pull request whose head is `branch`
    async fn find(&self, clone: &Path, branch: &str) -> Result<Option<PullRequest>>;

    async fn create_draft(
        &self,
        clone: &Path,
        branch: &str,
        base: &str,
        title: &str,
        body: &str,
    ) -> Result<PullRequest>;

    /// coeus's section of the body replaced, when it differs
    async fn edit_body_section(
        &self,
        clone: &Path,
        pull: &mut PullRequest,
        section: &str,
    ) -> Result<()>;

    /// out of draft
    async fn ready(&self, clone: &Path, pull: &mut PullRequest) -> Result<()>;
}

/// how the machine was reached
#[derive(Clone, Debug)]
pub struct Reached {
    pub platform: Platform,
    /// what the reach row says
    pub detail: String,
    /// as an earlier run left it: a key pinned before, or coeusd answering
    pub kept: bool,
}

/// nix on the machine, ready for nix-darwin
#[derive(Clone, Debug, Default)]
pub struct NixReady {
    /// the version that answers; `None` when coeusd answered instead of ssh
    pub version: Option<String>,
    /// installed or repaired by this run
    pub installed: bool,
    /// the `/etc` files moved to `.before-nix-darwin`
    pub moved: Vec<String>,
}

/// the machine's coeusd, as it answered
#[derive(Clone, Debug)]
pub struct Answer {
    pub version: String,
    /// the address its name resolved to
    pub address: String,
    pub reboot_required: bool,
    pub full_disk_access: FdaStatus,
}
