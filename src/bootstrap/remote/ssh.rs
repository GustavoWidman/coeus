//! the system's `ssh`, so the person's config, agent and jump hosts apply,
//! with what coeus decides:
//!
//! - one control master per remote user under the state directory
//!   (`ControlPersist=120`), reused by every command and by nix's own ssh
//!   (`NIX_SSHOPTS`), so a password login is typed once;
//! - `HostKeyAlgorithms=ssh-ed25519`, so the key ssh verifies is the key
//!   agenix reads;
//! - a `known_hosts` of its own holding exactly the trusted key under a
//!   `HostKeyAlias`, with `StrictHostKeyChecking=yes`, and `BatchMode=yes`
//!   once the first contact is over. nothing else may vouch for a key: no
//!   global file, `KnownHostsCommand` or DNS.

use std::{
    path::{Path, PathBuf},
    process::{ExitStatus, Stdio},
};

use eyre::{Result, bail, eyre};
use tokio::process::Command;

use super::{SessionKey, private, quote};
use crate::bootstrap::{SshKey, state::StateDir};

#[derive(Clone, Debug)]
pub struct Ssh {
    destination: String,
    /// the destination's user replaced by root
    root: bool,
    /// `-i`, the session key, with `IdentitiesOnly=yes`
    identity: Option<PathBuf>,
    known_hosts: PathBuf,
    control_path: PathBuf,
    /// the host key's name in `known_hosts`, the machine's fleet name
    alias: String,
}

impl Ssh {
    /// as the person's login, against the pinned key
    pub fn login(state: &StateDir, destination: &str) -> Self {
        Self {
            destination: destination.to_owned(),
            root: false,
            identity: None,
            known_hosts: state.known_hosts(),
            control_path: state.control_path(),
            alias: state.host().to_string(),
        }
    }

    /// the same machine as root, authenticated by the session key
    pub fn as_root(&self, session: &SessionKey) -> Self {
        Self {
            root: true,
            identity: Some(session.private().to_owned()),
            ..self.clone()
        }
    }

    pub const fn is_root(&self) -> bool {
        self.root
    }

    /// the key `destination` presents, learned without authenticating:
    /// `PreferredAuthentications=none` against a scratch known_hosts, which
    /// ssh fills before failing at login
    pub async fn first_contact(destination: &str) -> Result<SshKey> {
        let scratch = tempfile::Builder::new()
            .prefix("coeus-first-contact")
            .tempdir()?;
        let known_hosts = scratch.path().join("known_hosts");
        let output = Command::new("ssh")
            .args(first_contact_args(destination, &known_hosts))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .output()
            .await
            .map_err(|error| eyre!("could not run ssh: {error}"))?;

        let recorded = std::fs::read_to_string(&known_hosts).unwrap_or_default();
        match known_key(&recorded, FIRST_CONTACT) {
            Some(key) => Ok(key),
            None => Err(eyre!(
                "{destination} presented no ed25519 host key: {}",
                stderr_tail(&output.stderr)
            )),
        }
    }

    /// trusts `key` for this machine from now on
    pub fn pin(state: &StateDir, key: &SshKey) -> Result<()> {
        let line = format!("{} {key}\n", state.host());
        private::write(&state.known_hosts(), line.as_bytes())
    }

    /// the key an earlier run trusted
    pub fn pinned(state: &StateDir) -> Result<Option<SshKey>> {
        match std::fs::read_to_string(state.known_hosts()) {
            Ok(text) => Ok(known_key(&text, state.host().as_str())),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    /// opens the control master. without `batch`, ssh may ask a login
    /// password on the terminal the console hands over
    pub async fn connect(&self, batch: bool) -> Result<()> {
        let mut command = Command::new("ssh");
        command
            .args(self.options(batch))
            .args(["-o", "ConnectTimeout=15", &self.target(), "true"])
            .stdout(Stdio::null())
            .kill_on_drop(true);
        let status = match batch {
            true => {
                let output = command
                    .stdin(Stdio::null())
                    .stderr(Stdio::piped())
                    .output()
                    .await?;
                if !output.status.success() {
                    bail!("ssh {}: {}", self.target(), stderr_tail(&output.stderr));
                }
                output.status
            }
            false => command.status().await?,
        };
        if !status.success() {
            bail!("ssh {} failed", self.target());
        }
        Ok(())
    }

    /// one command on the machine through the master, its argv quoted for
    /// the remote shell
    pub fn command(&self, argv: &[String]) -> Command {
        let mut command = Command::new("ssh");
        command
            .args(self.options(true))
            .args(["-T", &self.target(), &quote::line(argv)])
            .kill_on_drop(true);
        command
    }

    /// the user ssh logs in as, after the person's config
    pub async fn user(&self) -> Result<String> {
        let output = Command::new("ssh")
            .args(self.options(true))
            .args(["-G", &self.target()])
            .output()
            .await?;
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .find_map(|line| line.strip_prefix("user "))
            .map(str::to_owned)
            .ok_or_else(|| eyre!("ssh -G {} names no user", self.target()))
    }

    /// `ssh-ng://<destination>` and the `NIX_SSHOPTS` that send nix through
    /// the same master and pin. `nix_daemon` is the daemon's path on the
    /// machine, which a non-interactive login on a Mac has not on `PATH`.
    pub fn nix_store(&self, nix_daemon: &str) -> (String, String) {
        let target = Destination::parse(&self.target());
        // one connection, so nix multiplexes over this master instead of
        // opening one of its own, which could not ask a password
        let url = format!(
            "ssh-ng://{}?max-connections=1&remote-program={nix_daemon}",
            target.authority
        );
        let mut options = self.options(true);
        if let Some(port) = target.port {
            options.extend(["-p".into(), port]);
        }
        (url, options.join(" "))
    }

    /// `ssh -O exit`, so no master outlives the run
    pub async fn close(&self) {
        let _ = Command::new("ssh")
            .args([
                "-o",
                &format!("ControlPath={}", self.control_path.display()),
            ])
            .args(["-O", "exit", &self.target()])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await;
    }

    /// ends every master an earlier run of this bootstrap left: one killed
    /// with `-9` leaves its master holding the channels of the commands it
    /// ran, and a command whose output nobody reads blocks on the machine,
    /// holding whatever it locked. the run's lock means none of them is in use.
    pub async fn end_stale(state: &StateDir) {
        let Ok(entries) = std::fs::read_dir(state.root()) else {
            return;
        };
        for entry in entries.flatten() {
            let is_master = entry.file_name().to_string_lossy().starts_with("ssh-")
                && entry
                    .file_type()
                    .is_ok_and(|kind| std::os::unix::fs::FileTypeExt::is_socket(&kind));
            if !is_master {
                continue;
            }
            let _ = Command::new("ssh")
                .args(["-o", &format!("ControlPath={}", entry.path().display())])
                .args(["-O", "exit", "stale"])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .await;
        }
    }

    /// whether a failure was the connection rather than the command: ssh
    /// exits 255 for its own errors
    pub fn lost(status: ExitStatus) -> bool {
        status.code() == Some(255)
    }

    /// the destination, with root as its user for the session key
    fn target(&self) -> String {
        match self.root {
            true => Destination::parse(&self.destination).with_user("root"),
            false => self.destination.clone(),
        }
    }

    fn options(&self, batch: bool) -> Vec<String> {
        let mut options: Vec<String> = [
            format!("HostKeyAlias={}", self.alias),
            format!("UserKnownHostsFile={}", self.known_hosts.display()),
            "GlobalKnownHostsFile=/dev/null".into(),
            "KnownHostsCommand=none".into(),
            "VerifyHostKeyDNS=no".into(),
            "StrictHostKeyChecking=yes".into(),
            "HostKeyAlgorithms=ssh-ed25519".into(),
            "UpdateHostKeys=no".into(),
            "ControlMaster=auto".into(),
            format!("ControlPath={}", self.control_path.display()),
            "ControlPersist=120".into(),
            "ServerAliveInterval=5".into(),
            "ServerAliveCountMax=3".into(),
            "LogLevel=ERROR".into(),
        ]
        .into_iter()
        .chain(batch.then(|| "BatchMode=yes".into()))
        .chain(self.identity.is_some().then(|| "IdentitiesOnly=yes".into()))
        .flat_map(|option| ["-o".into(), option])
        .collect();
        if let Some(identity) = &self.identity {
            options.extend(["-i".into(), identity.display().to_string()]);
        }
        options
    }
}

/// the alias first contact records the presented key under
const FIRST_CONTACT: &str = "coeus-first-contact";

/// an ssh that learns the host key and authenticates with nothing. the
/// scratch entry is read back by its alias, so it is written in the clear
/// whatever the person's config says about hashing
pub(super) fn first_contact_args(destination: &str, known_hosts: &Path) -> Vec<String> {
    [
        "BatchMode=yes",
        "PreferredAuthentications=none",
        "StrictHostKeyChecking=accept-new",
        &format!("UserKnownHostsFile={}", known_hosts.display()),
        "GlobalKnownHostsFile=/dev/null",
        "HashKnownHosts=no",
        "KnownHostsCommand=none",
        "VerifyHostKeyDNS=no",
        "HostKeyAlgorithms=ssh-ed25519",
        &format!("HostKeyAlias={FIRST_CONTACT}"),
        "ControlPath=none",
        "ConnectTimeout=15",
    ]
    .into_iter()
    .flat_map(|option| ["-o".to_owned(), option.to_owned()])
    .chain([destination.to_owned(), "true".to_owned()])
    .collect()
}

/// the ed25519 key a known_hosts file holds for `alias`
pub(super) fn known_key(text: &str, alias: &str) -> Option<SshKey> {
    text.lines().find_map(|line| {
        let (name, key) = line.trim().split_once(char::is_whitespace)?;
        (name == alias).then(|| SshKey::parse(key).ok()).flatten()
    })
}

/// the last line ssh said, for an error
pub(super) fn stderr_tail(stderr: &[u8]) -> String {
    let text = String::from_utf8_lossy(stderr);
    let said = |line: &&str| !line.trim().is_empty();
    // a program that explains itself after its error (clap's "try --help",
    // eyre's location) puts the reason on the error line, not the last one
    let error = |line: &&str| {
        let line = line.trim_start().to_ascii_lowercase();
        line.starts_with("error:") || line.starts_with("error ")
    };
    text.lines()
        .rev()
        .find(error)
        .or_else(|| text.lines().rev().find(said))
        .unwrap_or("it said nothing")
        .trim()
        .to_owned()
}

/// `[ssh://][user@]host[:port]`, the forms ssh takes, split far enough to
/// put another user in and hand nix its port
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Destination {
    uri: bool,
    /// `user@host` or `host`
    authority: String,
    host: String,
    port: Option<String>,
}

impl Destination {
    pub(super) fn parse(destination: &str) -> Self {
        let (uri, rest) = match destination.strip_prefix("ssh://") {
            Some(rest) => (true, rest.trim_end_matches('/')),
            None => (false, destination),
        };
        let (authority, port) = match uri {
            true => match rest.rsplit_once(':') {
                Some((authority, port))
                    if !port.is_empty()
                        && port.bytes().all(|byte| byte.is_ascii_digit())
                        && (!authority.contains('[') || authority.ends_with(']')) =>
                {
                    (authority, Some(port.to_owned()))
                }
                _ => (rest, None),
            },
            false => (rest, None),
        };
        let host = authority
            .rsplit_once('@')
            .map_or(authority, |(_, host)| host);
        Self {
            uri,
            authority: authority.to_owned(),
            host: host.to_owned(),
            port,
        }
    }

    pub(super) fn with_user(&self, user: &str) -> String {
        let authority = format!("{user}@{}", self.host);
        match (self.uri, &self.port) {
            (true, Some(port)) => format!("ssh://{authority}:{port}"),
            (true, None) => format!("ssh://{authority}"),
            (false, _) => authority,
        }
    }
}
