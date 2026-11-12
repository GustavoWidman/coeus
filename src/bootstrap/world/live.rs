//! the machine through ssh and coeusd

use std::sync::OnceLock;

use eyre::{Result, eyre};

use super::{Answer, Machine, NixReady, Reached};
use crate::{
    bootstrap::{
        Bootstrap,
        fleet::{self, Answered, Reboot},
        remote::{self, NixState, Probed, SessionKey, Ssh, Target},
        switch::{self, Release, Switched},
    },
    cli::Console,
};

/// the machine as a real run reaches it
#[derive(Default)]
pub struct Live {
    target: OnceLock<Target>,
    answered: OnceLock<Answered>,
}

impl Live {
    fn target(&self) -> Result<&Target> {
        self.target
            .get()
            .ok_or_else(|| eyre!("the machine was not reached yet"))
    }
}

impl Machine for Live {
    async fn reach(&self, bootstrap: &Bootstrap, console: &Console) -> Result<Reached> {
        let pinned = Ssh::pinned(&bootstrap.state)?.is_some();
        let target = remote::reach(bootstrap, console).await?;
        let platform = match (target.opened(), &target.daemon) {
            (Some(stock), _) => stock.probed.facts.platform.into(),
            (None, Some(daemon)) => daemon.hello.platform,
            (None, None) => return Err(eyre!("{} answered neither ssh nor coeusd", target.host)),
        };
        let reached = Reached {
            platform,
            detail: target.describe(),
            kept: pinned || target.daemon.is_some(),
        };
        let _ = self.target.set(target);
        Ok(reached)
    }

    async fn probed(&self, bootstrap: &Bootstrap, console: &Console) -> Result<Probed> {
        let stock = self.target()?.stock(bootstrap, console).await?;
        Ok(stock.probed.clone())
    }

    async fn nix(&self, bootstrap: &Bootstrap, console: &Console) -> Result<NixReady> {
        let target = self.target()?;
        if target.daemon.is_some() && target.opened().is_none() {
            return Ok(NixReady::default());
        }
        let stock = target.stock(bootstrap, console).await?;
        let installed = !matches!(stock.probed.nix, NixState::Installed { .. });
        let version = remote::converge_nix(stock, console).await?;
        let moved = remote::move_etc_aside(stock).await?;
        Ok(NixReady {
            version: Some(version),
            installed,
            moved,
        })
    }

    async fn switch(
        &self,
        release: &Release,
        bootstrap: &Bootstrap,
        console: &Console,
    ) -> Result<Switched> {
        switch::converge(release, self.target()?, bootstrap, console).await
    }

    async fn answer(&self, switched: &Switched, console: &Console) -> Result<Answer> {
        let answered = fleet::answer(self.target()?, switched, console).await?;
        let answer = Answer {
            version: answered.hello.version.clone(),
            address: answered.address.clone(),
            reboot_required: answered.view.system.reboot_required,
            full_disk_access: answered.view.system.fda,
        };
        let _ = self.answered.set(answered);
        Ok(answer)
    }

    async fn reboot(&self, _: &Answer, console: &Console) -> Result<Reboot> {
        let answered = self
            .answered
            .get()
            .ok_or_else(|| eyre!("coeusd did not answer yet"))?;
        fleet::reboot(answered, console).await
    }

    async fn cleanup(&self, bootstrap: &Bootstrap, console: &Console) -> Result<bool> {
        let state = &bootstrap.state;
        let Some(session) = SessionKey::existing(state)? else {
            return Ok(false);
        };
        let target = self.target()?;
        let stock = match target.opened() {
            Some(stock) => stock,
            None if refused(bootstrap, &session).await => {
                state.drop_session_key()?;
                return Ok(true);
            }
            None => target.stock(bootstrap, console).await?,
        };
        session.remove(stock).await?;
        target.close().await;
        state.drop_session_key()?;
        Ok(true)
    }
}

/// root turning the session key away: a run that took it out stopped
/// before forgetting it here
async fn refused(bootstrap: &Bootstrap, session: &SessionKey) -> bool {
    let Some(destination) = &bootstrap.saved.destination else {
        return false;
    };
    let root = Ssh::login(&bootstrap.state, destination).as_root(session);
    match root.connect(true).await {
        Ok(()) => false,
        Err(error) => format!("{error:#}").contains("Permission denied"),
    }
}
