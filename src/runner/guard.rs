use std::{
    env,
    net::{Ipv4Addr, SocketAddr},
    path::{Path, PathBuf},
    time::Duration,
};

use serde::Deserialize;
use tokio::{net::TcpStream, process::Command, time};

use crate::job::Guard;

/// what one round of health checks found after activating
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Health {
    /// coeusd answers on its port, so the fleet can still reach the machine
    pub daemon: bool,
    /// tailscale is running, or `None` when it is not checked
    pub tailscale: Option<bool>,
}

impl Health {
    pub fn is_healthy(&self) -> bool {
        self.daemon && self.tailscale != Some(false)
    }

    /// what is wrong, for people
    pub fn problem(&self) -> String {
        let mut problems = Vec::new();
        if !self.daemon {
            problems.push("coeusd does not answer");
        }
        if self.tailscale == Some(false) {
            problems.push("tailscale is not running");
        }
        problems.join(" and ")
    }
}

impl Guard {
    const INTERVAL: Duration = Duration::from_secs(2);
    const CHECK: Duration = Duration::from_secs(5);

    /// checks the machine until it is healthy or the timeout passes, and says
    /// what was still wrong then
    pub async fn verify(&self) -> Result<(), String> {
        let deadline = time::Instant::now() + self.timeout;

        loop {
            let health = self.check().await;
            if health.is_healthy() {
                return Ok(());
            }
            if time::Instant::now() + Self::INTERVAL > deadline {
                return Err(format!(
                    "{} {:?} after activating",
                    health.problem(),
                    self.timeout
                ));
            }
            time::sleep(Self::INTERVAL).await;
        }
    }

    pub async fn check(&self) -> Health {
        let address = SocketAddr::from((Ipv4Addr::LOCALHOST, self.daemon_port));
        let daemon = time::timeout(Self::CHECK, TcpStream::connect(address))
            .await
            .is_ok_and(|connected| connected.is_ok());

        let tailscale = match self.tailscale {
            true => match find("tailscale") {
                Some(tailscale) => Some(tailscale_running(&tailscale).await),
                None => None,
            },
            false => None,
        };

        Health { daemon, tailscale }
    }
}

async fn tailscale_running(program: &Path) -> bool {
    let status = Command::new(program)
        .args(["status", "--json"])
        .kill_on_drop(true)
        .output();

    match time::timeout(Guard::CHECK, status).await {
        Ok(Ok(output)) => is_running(&output.stdout),
        _ => false,
    }
}

/// whether `tailscale status --json` says the backend is running
pub fn is_running(status: &[u8]) -> bool {
    #[derive(Deserialize)]
    struct Status {
        #[serde(rename = "BackendState")]
        backend_state: String,
    }

    serde_json::from_slice::<Status>(status).is_ok_and(|status| status.backend_state == "Running")
}

fn find(program: &str) -> Option<PathBuf> {
    env::split_paths(&env::var_os("PATH")?)
        .map(|dir| dir.join(program))
        .find(|path| path.is_file())
}
