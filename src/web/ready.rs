//! whether this host can serve the dashboard: tailscale in the tailnet and
//! in touch with its coordination server and relays, and the local coeusd
//! answering. no other machine is asked, so the last host standing keeps
//! serving.

use std::future::Future;

use log::{info, warn};
use tokio::{
    sync::watch,
    time::{MissedTickBehavior, interval, timeout},
};

use crate::{
    client::{CoeusClient, Tailnet},
    config::{ClientSettings, Endpoint, Key, ReadinessSettings},
};

/// consecutive probes since readiness last changed
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Readiness {
    Down { passes: u8 },
    Up { fails: u8 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    Rose,
    Fell,
}

impl Readiness {
    pub const START: Self = Self::Down { passes: 0 };

    /// up after `up_after` passes in a row, down after `down_after` fails in
    /// a row
    pub fn step(self, passed: bool, settings: &ReadinessSettings) -> (Self, Option<Edge>) {
        match (self, passed) {
            (Self::Down { passes }, true) => {
                let passes = passes.saturating_add(1);
                if passes >= settings.up_after.get() {
                    (Self::Up { fails: 0 }, Some(Edge::Rose))
                } else {
                    (Self::Down { passes }, None)
                }
            }
            (Self::Down { .. }, false) => (Self::START, None),
            (Self::Up { .. }, true) => (Self::Up { fails: 0 }, None),
            (Self::Up { fails }, false) => {
                let fails = fails.saturating_add(1);
                if fails >= settings.down_after.get() {
                    (Self::START, Some(Edge::Fell))
                } else {
                    (Self::Up { fails }, None)
                }
            }
        }
    }

    pub const fn is_up(self) -> bool {
        matches!(self, Self::Up { .. })
    }
}

/// runs `probe` every interval, each within the interval, and keeps `ready`
/// saying whether the host is ready, until nobody listens
pub async fn watch<P, F>(settings: ReadinessSettings, mut probe: P, ready: watch::Sender<bool>)
where
    P: FnMut() -> F,
    F: Future<Output = Result<(), String>>,
{
    let mut state = Readiness::START;
    let mut trouble = None;
    let mut ticks = interval(settings.interval);
    ticks.set_missed_tick_behavior(MissedTickBehavior::Delay);

    while !ready.is_closed() {
        ticks.tick().await;
        let verdict = timeout(settings.interval, probe())
            .await
            .unwrap_or_else(|_| Err(format!("no answer within {:?}", settings.interval)));

        if let Err(reason) = &verdict
            && trouble.as_ref() != Some(reason)
        {
            warn!("not ready to serve: {reason}");
        }
        trouble = verdict.err();

        let (next, edge) = state.step(trouble.is_none(), &settings);
        state = next;
        match edge {
            Some(Edge::Rose) => info!("ready to serve"),
            Some(Edge::Fell) => warn!("no longer ready to serve"),
            None => continue,
        }
        ready.send_replace(state.is_up());
    }
}

/// passes when [Tailnet::trouble] finds none and the coeusd at `daemon`
/// shows its view to `key`
pub async fn probe(client: &ClientSettings, daemon: &Endpoint, key: &Key) -> Result<(), String> {
    let (tailnet, answered) = tokio::join!(Tailnet::read(), answers(client, daemon, key));
    match tailnet {
        None => Err("tailscale does not answer".to_owned()),
        Some(tailnet) => tailnet.trouble().map_or(answered, Err),
    }
}

/// passes when the coeusd at `daemon` shows its view to `key`
pub async fn answers(client: &ClientSettings, daemon: &Endpoint, key: &Key) -> Result<(), String> {
    let mut coeusd = CoeusClient::connect(client, daemon, key)
        .await
        .map_err(|unreach| format!("the local coeusd is unreachable: {unreach}"))?;
    coeusd
        .view()
        .await
        .map(drop)
        .map_err(|error| format!("the local coeusd did not answer: {error:#}"))
}
