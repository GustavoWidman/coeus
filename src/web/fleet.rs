//! one live copy of the fleet, built from every machine's views by a single
//! task, the aggregator, and read by every browser stream

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use tokio::sync::{mpsc, watch};

use super::{
    derive,
    diffs::Diffs,
    view::{DeploymentView, DiffSummary, LinkView, LiveProgress, MachineCard, MachineState},
};
use crate::{
    client::{self, Link},
    common::model::{Hostname, MachineView, Timestamp},
};

/// a value and the seq of its last change
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Versioned<T> {
    pub rev: u64,
    pub value: T,
}

/// the fleet as browsers see it. every entity carries the seq of its last
/// change, so a browser that has seen up to some seq needs only the entities
/// changed since
#[derive(Clone, Debug)]
pub struct FleetState {
    /// drawn when the process starts: seqs mean nothing to another process
    pub epoch: u64,
    /// raised on every change of any entity
    pub seq: u64,
    /// in inventory order
    pub machines: Vec<Versioned<MachineCard>>,
    /// `None` once the job whose progress it was ends
    pub progress: BTreeMap<Hostname, Versioned<Option<LiveProgress>>>,
    /// by order; `None` once a deployment leaves the dashboard
    pub deployments: BTreeMap<String, Versioned<Option<DeploymentView>>>,
    /// removals this old are forgotten: a browser that saw no later seq may
    /// have missed one, so it starts over from a snapshot
    pub floor: u64,
}

impl FleetState {
    /// removed deployments remembered, so browsers that saw them can be told
    const TOMBSTONES: usize = 32;

    fn new(hosts: &[Hostname]) -> Self {
        Self {
            epoch: rand::random(),
            seq: hosts.len() as u64,
            machines: hosts
                .iter()
                .zip(1..)
                .map(|(host, rev)| Versioned {
                    rev,
                    value: MachineCard {
                        host: host.clone(),
                        link: LinkView::Connecting,
                        state: None,
                    },
                })
                .collect(),
            progress: BTreeMap::new(),
            deployments: BTreeMap::new(),
            floor: 0,
        }
    }

    /// whether a browser that saw everything up to `seq` of `epoch` can be
    /// brought up to date with the changes since alone
    pub fn resumes(&self, epoch: u64, seq: u64) -> bool {
        epoch == self.epoch && (self.floor..=self.seq).contains(&seq)
    }

    fn bump(&mut self) -> u64 {
        self.seq += 1;
        self.seq
    }

    fn forget_tombstones(&mut self) {
        let mut tombstones: Vec<(u64, String)> = self
            .deployments
            .iter()
            .filter(|(_, deployment)| deployment.value.is_none())
            .map(|(order, deployment)| (deployment.rev, order.clone()))
            .collect();
        if tombstones.len() <= Self::TOMBSTONES {
            return;
        }

        tombstones.sort_unstable();
        for (rev, order) in &tombstones[..tombstones.len() - Self::TOMBSTONES] {
            self.deployments.remove(order);
            self.floor = self.floor.max(*rev);
        }
    }
}

/// what the aggregator learns
#[derive(Debug)]
pub enum Input {
    /// what a machine's daemon said, or how reaching it goes, as of `at`
    Fleet {
        at: Timestamp,
        event: client::FleetEvent,
    },
    /// a closure diff was read into the cache
    Diffed,
}

/// the only writer of [FleetState]
pub struct Aggregator {
    state: FleetState,
    /// when each machine's last view arrived, which is when it was last seen
    viewed: BTreeMap<Hostname, Timestamp>,
    /// the latest time any input carried
    now: Timestamp,
    diffs: Arc<Diffs>,
}

impl Aggregator {
    pub fn new(hosts: &[Hostname], diffs: Arc<Diffs>) -> Self {
        Self {
            state: FleetState::new(hosts),
            viewed: BTreeMap::new(),
            now: Timestamp(0),
            diffs,
        }
    }

    pub fn state(&self) -> &FleetState {
        &self.state
    }

    /// applies inputs as they come and publishes the state after each batch,
    /// until every sender is gone
    pub async fn run(
        mut self,
        mut inputs: mpsc::Receiver<Input>,
        published: watch::Sender<Arc<FleetState>>,
    ) {
        while let Some(input) = inputs.recv().await {
            let mut changed = self.apply(input);
            while let Ok(input) = inputs.try_recv() {
                changed |= self.apply(input);
            }
            if changed {
                published.send_replace(Arc::new(self.state.clone()));
            }
        }
    }

    /// whether anything a browser sees changed
    pub fn apply(&mut self, input: Input) -> bool {
        let seq = self.state.seq;
        match input {
            Input::Fleet { at, event } => {
                self.now = self.now.max(at);
                let changed = match event {
                    client::FleetEvent::View { host, view } => self.view(&host, at, &view),
                    client::FleetEvent::Link { host, link } => self.link(&host, at, &link),
                };
                if changed {
                    self.deployments();
                }
            }
            Input::Diffed => self.deployments(),
        }
        self.state.seq != seq
    }

    fn card(&mut self, host: &Hostname) -> Option<usize> {
        self.state
            .machines
            .iter()
            .position(|card| card.value.host == *host)
    }

    /// whether the machine's card changed
    fn view(&mut self, host: &Hostname, at: Timestamp, view: &MachineView) -> bool {
        let Some(at_card) = self.card(host) else {
            return false;
        };
        self.viewed.insert(host.clone(), at);

        let progress = derive::progress(view);
        let changed = match self.state.progress.get(host) {
            Some(known) => known.value != progress,
            None => progress.is_some(),
        };
        if changed {
            let rev = self.state.bump();
            self.state.progress.insert(
                host.clone(),
                Versioned {
                    rev,
                    value: progress,
                },
            );
        }

        // a view arrives several times a second, so the card changes only
        // when more than the time it was seen does
        let state = derive::state(view, at);
        let card = &self.state.machines[at_card].value;
        if card.state.as_ref().is_some_and(|known| same(known, &state)) {
            return false;
        }
        let rev = self.state.bump();
        let card = &mut self.state.machines[at_card];
        card.rev = rev;
        card.value.state = Some(state);
        true
    }

    /// whether the machine's card changed
    fn link(&mut self, host: &Hostname, at: Timestamp, link: &Link) -> bool {
        let Some(at_card) = self.card(host) else {
            return false;
        };
        let card = &self.state.machines[at_card].value;
        let since = match (&card.link, link) {
            (LinkView::Unreachable { since, .. }, Link::Unreachable(_)) => *since,
            _ => at,
        };
        let link = derive::link(link, since);
        if card.link == link {
            return false;
        }

        let rev = self.state.bump();
        let seen = self.viewed.get(host).copied();
        let card = &mut self.state.machines[at_card];
        card.rev = rev;
        card.value.link = link;
        if let (Some(state), Some(seen)) = (&mut card.value.state, seen) {
            state.seen_at = seen;
        }
        true
    }

    /// derives the deployments again and stamps the ones that changed
    fn deployments(&mut self) {
        let cards: Vec<MachineCard> = self
            .state
            .machines
            .iter()
            .map(|card| card.value.clone())
            .collect();
        let mut current: BTreeMap<String, DeploymentView> = derive::deployments(&cards, self.now)
            .into_iter()
            .map(|mut deployment| {
                for row in &mut deployment.rows {
                    row.diff = row
                        .job
                        .and_then(|job| self.diffs.cached(&row.host, job))
                        .flatten()
                        .map(|diff| DiffSummary::from(&*diff));
                }
                (deployment.order.clone(), deployment)
            })
            .collect();

        let orders: BTreeSet<String> = self
            .state
            .deployments
            .keys()
            .chain(current.keys())
            .cloned()
            .collect();
        for order in orders {
            let value = current.remove(&order);
            let changed = match self.state.deployments.get(&order) {
                Some(known) => known.value != value,
                None => value.is_some(),
            };
            if changed {
                let rev = self.state.bump();
                self.state
                    .deployments
                    .insert(order, Versioned { rev, value });
            }
        }
        self.state.forget_tombstones();
    }
}

/// whether two states of a machine differ only in when it was seen
fn same(known: &MachineState, new: &MachineState) -> bool {
    MachineState {
        seen_at: new.seen_at,
        ..known.clone()
    } == *new
}
