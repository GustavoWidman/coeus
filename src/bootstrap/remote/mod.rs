//! the machine being bootstrapped, through whichever door is open: ssh
//! while it runs its stock system, coeusd over the tailnet once it runs the
//! fleet's. after the first switch the stock user may be gone, so a run
//! where coeusd answers never opens ssh unless a step asks for it.

mod call;
mod exec;
mod facts;
mod nix;
mod private;
mod probe;
mod quote;
mod running;
mod session;
mod ssh;
mod sudo;

#[cfg(test)]
pub(crate) mod test;

use eyre::{Result, WrapErr, bail, eyre};
use tokio::sync::OnceCell;

pub use call::{AdoptCall, Call};
pub use facts::{BootLoader, Facts, Platform, System};
pub use nix::{NixState, converge as converge_nix, move_etc_aside};
pub use private::write as write_private;
pub use probe::Probed;
pub use running::Running;
pub use session::SessionKey;
pub use ssh::Ssh;
pub use sudo::Privilege;

use crate::{
    bootstrap::{Bootstrap, SshKey, Step, fleet::Daemon},
    cli::Console,
    common::model::Hostname,
    config::ClientSettings,
    ui::{
        ask::{Answer, Prompt},
        checklist::Mark,
    },
};

pub struct Target {
    pub host: Hostname,
    /// coeusd answered as `host` with the fleet key
    pub daemon: Option<Daemon>,
    /// ssh, opened on first use
    stock: OnceCell<Stock>,
    /// how ssh reaches the stock machine, when a run was told
    destination: Option<String>,
    settings: ClientSettings,
}

/// the machine as its stock system shows it over ssh, as root through the
/// session key once it is installed
pub struct Stock {
    pub ssh: Ssh,
    pub privilege: Privilege,
    pub probed: Probed,
}

/// asks the machine's coeusd first; when it does not answer, ssh must, and
/// a first contact asks the person to trust the key it presents
pub async fn reach(bootstrap: &Bootstrap, console: &Console) -> Result<Target> {
    let host = &bootstrap.host;
    console.mark(
        Step::Reach,
        Mark::Active,
        format!("asking {host}'s coeusd over the tailnet"),
    );
    let target = Target {
        host: host.clone(),
        daemon: Daemon::connect(&bootstrap.settings, host).await?,
        stock: OnceCell::new(),
        destination: bootstrap.saved.destination.clone(),
        settings: bootstrap.settings.clone(),
    };
    if target.daemon.is_none() {
        target.stock(bootstrap, console).await?;
    }
    Ok(target)
}

impl Target {
    /// the stock machine over ssh, opened now if it was not
    pub async fn stock(&self, bootstrap: &Bootstrap, console: &Console) -> Result<&Stock> {
        self.stock
            .get_or_try_init(|| self.open(bootstrap, console))
            .await
    }

    /// the stock machine, if a step opened ssh already
    pub fn opened(&self) -> Option<&Stock> {
        self.stock.get()
    }

    pub const fn settings(&self) -> &ClientSettings {
        &self.settings
    }

    /// what the reach row says: the machine, or the coeusd that answered
    pub fn describe(&self) -> String {
        match (self.stock.get(), &self.daemon) {
            (Some(stock), _) => format!("{} · host key pinned", stock.probed.facts.describe()),
            (None, Some(daemon)) => format!(
                "coeusd {} answers on {} over the tailnet",
                daemon.hello.version, self.host
            ),
            (None, None) => self.host.to_string(),
        }
    }

    /// ends the ssh master, if one was opened
    pub async fn close(&self) {
        if let Some(stock) = self.stock.get() {
            stock.ssh.close().await;
        }
    }

    async fn open(&self, bootstrap: &Bootstrap, console: &Console) -> Result<Stock> {
        let Some(destination) = &self.destination else {
            bail!(
                "{} has no coeusd that answers, and coeus does not know how to reach it over \
                 ssh: say how with --to user@address",
                self.host
            );
        };
        let state = &bootstrap.state;
        let pin = self.pin(bootstrap, destination, console).await?;
        let login = Ssh::login(state, destination);
        let login_user = login.user().await?;
        let session = SessionKey::ensure(state)?;
        let root = login.as_root(&session);

        console.mark(
            Step::Reach,
            Mark::Active,
            format!("connecting to {destination}"),
        );
        let (ssh, privilege) = match root.connect(true).await {
            Ok(()) => (root, Privilege::Root),
            Err(_) => {
                log_in(&login, &login_user, destination, console).await?;
                let privilege = Privilege::establish(&login, &login_user, console).await?;
                session.install(&login, &privilege).await?;
                match root.connect(true).await {
                    Ok(()) => {
                        login.close().await;
                        (root, Privilege::Root)
                    }
                    Err(error) => {
                        console.note(format!(
                            "root on {} does not take coeus's session key ({error:#}); \
                             staying with {login_user} and sudo",
                            self.host
                        ));
                        (login, privilege)
                    }
                }
            }
        };

        console.mark(Step::Reach, Mark::Active, format!("probing {}", self.host));
        let probed = probe(&ssh, &privilege, &login_user).await?;
        if probed.host_key != pin {
            bail!(
                "sshd on {destination} presents {}, but its /etc/ssh/ssh_host_ed25519_key.pub \
                 holds {}: the key enrolled must be the key agenix reads there",
                pin.fingerprint(),
                probed.host_key.fingerprint()
            );
        }
        Ok(Stock {
            ssh,
            privilege,
            probed,
        })
    }

    /// the pinned key, or the one first contact reads, trusted by the
    /// person or by `--host-key`
    async fn pin(
        &self,
        bootstrap: &Bootstrap,
        destination: &str,
        console: &Console,
    ) -> Result<SshKey> {
        let state = &bootstrap.state;
        let expected = bootstrap.options.host_key.as_deref();
        if let Some(pinned) = Ssh::pinned(state)? {
            if let Some(expected) = expected
                && pinned.fingerprint() != expected
            {
                bail!(
                    "{} is pinned to {}, not {expected}: `coeus bootstrap {} --forget` drops \
                     the pin",
                    self.host,
                    pinned.fingerprint(),
                    self.host
                );
            }
            return Ok(pinned);
        }

        console.mark(
            Step::Reach,
            Mark::Active,
            format!("reading the host key {destination} presents"),
        );
        let key = Ssh::first_contact(destination).await?;
        let fingerprint = key.fingerprint();
        match expected {
            Some(expected) if expected == fingerprint => {}
            Some(expected) => {
                return Err(eyre!(
                    "{destination} presents {fingerprint}, not the {expected} --host-key names"
                ));
            }
            None => {
                let question = format!(
                    "{destination} presents a host key coeus has never seen\n\
                     ed25519 {fingerprint}\n\
                     its console prints the same for `ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub`\n\
                     trust it and enroll it as {}'s key?",
                    self.host
                );
                let answer = console
                    .ask(Prompt::confirm(question, true))
                    .await
                    .wrap_err_with(|| {
                        format!("pass --host-key {fingerprint} to trust it without a terminal")
                    })?;
                match answer {
                    Answer::Value(yes) if yes == "true" => {}
                    Answer::Cancelled => {
                        return Err(eyre!("cancelled before trusting the host key"));
                    }
                    _ => return Err(eyre!("{destination}'s host key was not trusted")),
                }
            }
        }
        Ssh::pin(state, &key)?;
        Ok(key)
    }
}

impl Stock {
    /// `call` on the machine, its stdout
    pub async fn run(&self, call: &Call) -> Result<String> {
        exec::text(&self.ssh, &self.privilege, call).await
    }

    /// `call` on the machine, its output whatever its status
    pub async fn output(&self, call: &Call) -> Result<std::process::Output> {
        exec::output(&self.ssh, &self.privilege, call).await
    }

    /// `call` on the machine, each line of its output to `line` as it comes
    pub async fn follow(
        &self,
        call: &Call,
        line: impl FnMut(&str),
    ) -> Result<std::process::ExitStatus> {
        exec::follow(&self.ssh, &self.privilege, call, line).await
    }

    /// `call` started on the machine, its stdout and stderr piped
    pub fn spawn(&self, call: &Call) -> Result<tokio::process::Child> {
        exec::spawn(&self.ssh, &self.privilege, call)
    }

    /// the machine as it is now
    pub async fn probe(&self) -> Result<Probed> {
        probe(&self.ssh, &self.privilege, &self.probed.facts.login_user).await
    }
}

/// the login's master: without a password when keys do, else on the
/// terminal, where ssh asks itself
async fn log_in(login: &Ssh, user: &str, destination: &str, console: &Console) -> Result<()> {
    if login.connect(true).await.is_ok() {
        return Ok(());
    }
    if !console.live {
        bail!(
            "ssh {destination} needs a password or a key passphrase, and there is no terminal \
             to type it on"
        );
    }
    let why = format!("ssh asks for {user}'s password on {destination}");
    console.hand_over(&why, login.connect(false)).await
}

async fn probe(ssh: &Ssh, privilege: &Privilege, login_user: &str) -> Result<Probed> {
    let call = Call::Probe {
        login_user: login_user.to_owned(),
    };
    let output = exec::text(ssh, privilege, &call).await?;
    let nixos = output.lines().any(|line| line == "osId=nixos");
    let hardware = match nixos {
        true => Some(exec::text(ssh, privilege, &Call::HardwareConfig).await?),
        false => None,
    };
    Probed::parse(&output, hardware)
}
