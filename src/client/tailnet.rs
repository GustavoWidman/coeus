use std::{collections::HashMap, net::IpAddr, time::Duration};

use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::{common::model::Timestamp, ui::format};

/// `tailscale status --json`, as much of it as says which machine answers to
/// a name, whether the coordination server sees it online, and whether this
/// machine is in touch with that server and its relays
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Tailnet {
    #[serde(default)]
    backend_state: String,
    #[serde(rename = "Self")]
    this: Option<Node>,
    #[serde(default)]
    peer: HashMap<String, Node>,
    /// tailscale's warnings, in its own words
    #[serde(default)]
    health: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Node {
    #[serde(rename = "DNSName", default)]
    dns_name: String,
    #[serde(rename = "TailscaleIPs", default)]
    tailscale_ips: Vec<IpAddr>,
    #[serde(default)]
    online: bool,
    last_seen: Option<DateTime<Utc>>,
}

/// a machine the coordination server sees offline: nothing reaches it until
/// it comes back, so dialing it only waits out the connect timeout
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Offline {
    /// when the coordination server last heard from it, unset if never
    pub last_seen: Option<Timestamp>,
}

impl Tailnet {
    /// how long tailscale may take to answer
    const PATIENCE: Duration = Duration::from_secs(5);

    /// what tailscale's warnings say when it cannot reach its coordination
    /// server or its home relay, or has no network at all. tailscale names
    /// them only in prose, so these are phrases from its `health` package
    /// (1.104), shared by every such warning and by no other.
    const CUT_OFF: [&str; 3] = ["coordination server", "relay server", "network is down"];

    /// the tailnet as this machine's tailscale sees it, `None` when no
    /// tailscale answers
    pub async fn read() -> Option<Self> {
        Self::parse(&tailscale(&["status", "--json"]).await?.stdout)
    }

    pub fn parse(status: &[u8]) -> Option<Self> {
        serde_json::from_slice(status).ok()
    }

    /// why this machine is not fully in the tailnet, `None` when tailscale
    /// runs, holds its long poll to the coordination server and warns about
    /// neither that server nor its relays. `Self.Online` is that poll, so it
    /// drops at once; tailscale holds back its warning about it for minutes.
    pub fn trouble(&self) -> Option<String> {
        if self.backend_state != "Running" {
            return Some(format!("tailscale is {}", self.backend_state));
        }
        if !self.this.as_ref().is_some_and(|this| this.online) {
            return Some("tailscale is out of touch with its coordination server".to_owned());
        }
        self.health
            .iter()
            .find(|warning| Self::CUT_OFF.iter().any(|phrase| warning.contains(phrase)))
            .map(|warning| format!("tailscale warns: {warning}"))
    }

    /// the first tailnet address of the machine MagicDNS names `host`
    pub fn address(&self, host: &str) -> Option<String> {
        let peer = self.peer.values().find(|peer| peer.named(host))?;
        peer.tailscale_ips.first().map(IpAddr::to_string)
    }

    /// the machine coeus dials at `host`, a MagicDNS name or a tailnet
    /// address, if the coordination server sees it offline. only a tailscale
    /// in touch with that server knows: one cut off from it keeps whatever it
    /// heard last, so it names nobody offline.
    pub fn offline(&self, host: &str) -> Option<Offline> {
        let current =
            self.backend_state == "Running" && self.this.as_ref().is_some_and(|this| this.online);
        if !current {
            return None;
        }
        let peer = self
            .peer
            .values()
            .find(|peer| peer.named(host) || peer.at(host))?;
        (!peer.online).then(|| Offline {
            last_seen: peer.last_seen(),
        })
    }
}

/// what the tailscale CLI says to `args`, wherever it is installed; `None`
/// when none answers in time
pub async fn tailscale(args: &[&str]) -> Option<std::process::Output> {
    for program in [
        "tailscale",
        "/Applications/Tailscale.app/Contents/MacOS/Tailscale",
    ] {
        let output = tokio::process::Command::new(program)
            .args(args)
            .kill_on_drop(true)
            .output();
        if let Ok(Ok(output)) = tokio::time::timeout(Tailnet::PATIENCE, output).await {
            return Some(output);
        }
    }
    None
}

impl Node {
    /// MagicDNS names it `host`, alone or with the tailnet's domain
    fn named(&self, host: &str) -> bool {
        let host = host.trim_end_matches('.');
        let name = self.dns_name.trim_end_matches('.');
        !name.is_empty() && name.eq_ignore_ascii_case(host)
            || name
                .split('.')
                .next()
                .is_some_and(|label| label.eq_ignore_ascii_case(host))
    }

    fn at(&self, host: &str) -> bool {
        host.parse::<IpAddr>()
            .is_ok_and(|host| self.tailscale_ips.contains(&host))
    }

    /// tailscale writes the zero time for a machine it never heard from
    fn last_seen(&self) -> Option<Timestamp> {
        self.last_seen
            .map(|at| at.timestamp_millis())
            .filter(|millis| *millis > 0)
            .map(Timestamp)
    }
}

impl Offline {
    pub fn describe(self, now: Timestamp) -> String {
        match self.last_seen {
            Some(at) => format!("offline, tailscale last saw it {}", format::ago(at, now)),
            None => "offline, as tailscale sees it".to_owned(),
        }
    }
}
