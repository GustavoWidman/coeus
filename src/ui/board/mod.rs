//! the fleet as the CLI sees it: one row per machine, folded from the
//! machines' views and from what this invocation ordered

mod eta;
mod render;
mod row;

pub use eta::eta;
pub use render::{Frame, Tally};
pub use row::{Ordering, Row, Status, Verdict};

use crate::{
    client::{FleetEvent, Link},
    common::model::{Action, Hostname, OrderId, Source, Timestamp},
};

pub struct Board {
    pub heading: Heading,
    pub rows: Vec<Row>,
}

/// what the board follows
#[derive(Clone, Debug)]
pub enum Heading {
    /// one order's jobs, as `coeus deploy` placed them
    Order {
        id: OrderId,
        source: Source,
        action: Action,
        since: Timestamp,
    },
    /// whatever each machine does, whoever ordered it
    Watch { since: Timestamp },
}

impl Heading {
    pub const fn since(&self) -> Timestamp {
        match self {
            Self::Order { since, .. } | Self::Watch { since } => *since,
        }
    }
}

impl Board {
    pub fn new(heading: Heading, hosts: &[Hostname]) -> Self {
        let ordering = match heading {
            Heading::Order { .. } => Ordering::Pending { wave: 1 },
            Heading::Watch { .. } => Ordering::Watching,
        };

        Self {
            heading,
            rows: hosts
                .iter()
                .map(|host| Row::new(host.clone(), ordering.clone()))
                .collect(),
        }
    }

    pub fn apply(&mut self, event: FleetEvent) {
        match event {
            FleetEvent::Link { host, link } => {
                if let Some(row) = self.row_mut(&host) {
                    row.link = link;
                }
            }
            FleetEvent::View { host, view } => {
                if let Some(row) = self.row_mut(&host) {
                    row.link = Link::Connected;
                    row.view = Some(*view);
                }
            }
        }
    }

    pub fn order(&mut self, host: &Hostname, ordering: Ordering) {
        if let Some(row) = self.row_mut(host) {
            row.ordering = ordering;
        }
    }

    pub fn row(&self, host: &Hostname) -> Option<&Row> {
        self.rows.iter().find(|row| &row.host == host)
    }

    fn row_mut(&mut self, host: &Hostname) -> Option<&mut Row> {
        self.rows.iter_mut().find(|row| &row.host == host)
    }

    /// every ordered machine reached a verdict
    pub fn is_settled(&self) -> bool {
        matches!(self.heading, Heading::Order { .. })
            && self.rows.iter().all(|row| row.verdict().is_some())
    }

    pub fn verdicts(&self) -> impl Iterator<Item = Verdict> + '_ {
        self.rows.iter().filter_map(Row::verdict)
    }
}
