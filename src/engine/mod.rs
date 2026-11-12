//! the deployment engine: one task that owns every job's lifecycle on this
//! machine and publishes what the live views show

mod actor;
mod follow;
mod handle;
mod host;
mod lane;
mod notify;
mod reconcile;
mod running;
mod upgrade;

#[cfg(test)]
mod test;

use std::{path::Path, sync::Arc};

use eyre::Result;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

pub use handle::EngineHandle;
pub use host::{Host, SystemFacts};

use crate::{config::DaemonSettings, job::JobStore, nix::FlakeRepo};

pub struct Engine;

impl Engine {
    /// recovers the jobs on disk and starts the engine task. `restart` is
    /// cancelled when the engine wants coeusd to exit into a newer version;
    /// `shutdown` stops the engine.
    pub async fn start<H: Host>(
        host: H,
        settings: DaemonSettings,
        config: &Path,
        restart: CancellationToken,
        shutdown: CancellationToken,
    ) -> Result<EngineHandle<H>> {
        let host = Arc::new(host);
        let store = JobStore::open(&settings.state_dir)?;
        let repo = FlakeRepo::new(settings.flake.clone());

        let upgrade = upgrade::UpgradeWatch::new(config);
        let (actor, view) = actor::Actor::load(host.clone(), settings, upgrade, restart).await?;

        let (commands, receiver) = mpsc::channel(64);
        tokio::spawn(actor.run(receiver, shutdown));

        Ok(EngineHandle::new(commands, view, store, host, repo))
    }
}
