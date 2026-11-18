use std::sync::Arc;

use eyre::{Result, eyre};
use tokio::sync::mpsc;

use super::{
    fixture::{self, Fixture},
    fleet::Input,
};
use crate::{
    client::{Attachment, AttachmentEvent, Fleet},
    common::model::{Hostname, JobId, JobSelector, Timestamp},
};

/// where the fleet's views and job logs come from
#[derive(Clone)]
pub enum Source {
    /// every machine's coeusd, reached with the read-only key
    Live(Fleet),
    /// a simulated fleet, whose clock can be moved
    Fixture(Arc<Fixture>),
}

/// one job's log, from some offset on
pub enum Feed {
    Live(Box<Attachment>),
    Fixture(fixture::Feed),
}

impl Source {
    /// the machines shown, in inventory order
    pub fn hosts(&self) -> Vec<Hostname> {
        match self {
            Self::Live(fleet) => fleet.hosts().to_vec(),
            Self::Fixture(fixture) => fixture.hosts(),
        }
    }

    /// hands the aggregator everything the machines say, until it stops
    /// listening
    pub fn feed(&self, inputs: mpsc::Sender<Input>) {
        match self {
            Self::Live(fleet) => {
                let mut events = fleet.watch();
                tokio::spawn(async move {
                    while let Some(event) = events.recv().await {
                        let input = Input::Fleet {
                            at: Timestamp::now(),
                            event,
                        };
                        if inputs.send(input).await.is_err() {
                            return;
                        }
                    }
                });
            }
            Self::Fixture(fixture) => fixture.feed(inputs),
        }
    }

    /// job `job`'s log on `host` from offset `from`, live until it ends
    pub async fn attach(&self, host: &Hostname, job: u64, from: u64) -> Result<Feed> {
        match self {
            Self::Live(fleet) => {
                if !fleet.hosts().contains(host) {
                    return Err(eyre!("{host} is not in the inventory"));
                }
                let client = fleet.connect(host).await?;
                let attachment = client.attach(JobSelector::Id(JobId(job)), from, None);
                Ok(Feed::Live(Box::new(attachment.await?)))
            }
            Self::Fixture(fixture) => Ok(Feed::Fixture(fixture.attach(host, job, from)?)),
        }
    }
}

impl Feed {
    /// the next piece of the log, or `None` once the machine hangs up
    pub async fn next(&mut self) -> Result<Option<AttachmentEvent>> {
        match self {
            Self::Live(attachment) => attachment.next().await,
            Self::Fixture(feed) => feed.next().await,
        }
    }
}
