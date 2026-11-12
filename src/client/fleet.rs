use std::{future::Future, sync::Arc, time::Duration};

use futures_util::future::join_all;
use tokio::sync::mpsc;

use super::{CoeusClient, Targets};
use crate::{
    common::model::{Hostname, MachineView},
    config::{ClientSettings, Key},
};

/// the selected machines, reached concurrently
#[derive(Clone)]
pub struct Fleet {
    settings: Arc<ClientSettings>,
    key: Arc<Key>,
    targets: Targets,
}

/// how a watched machine's connection is doing
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Link {
    Connecting,
    Connected,
    Unreachable(String),
}

#[derive(Debug)]
pub enum FleetEvent {
    Link { host: Hostname, link: Link },
    View { host: Hostname, view: Box<MachineView> },
}

impl Fleet {
    const BACKOFF: [Duration; 4] = [
        Duration::from_secs(1),
        Duration::from_secs(2),
        Duration::from_secs(5),
        Duration::from_secs(10),
    ];

    pub fn new(settings: ClientSettings, key: Key, targets: Targets) -> Self {
        Self {
            settings: Arc::new(settings),
            key: Arc::new(key),
            targets,
        }
    }

    pub fn hosts(&self) -> &[Hostname] {
        self.targets.hosts()
    }

    pub async fn connect(&self, host: &Hostname) -> eyre::Result<CoeusClient> {
        let endpoint = self.settings.endpoint(host);
        CoeusClient::connect(&self.settings, &endpoint, &self.key).await
    }

    /// runs `operation` against every machine at once, in target order
    pub async fn each<T, F, Fut>(&self, operation: F) -> Vec<(Hostname, eyre::Result<T>)>
    where
        F: Fn(Hostname, CoeusClient) -> Fut,
        Fut: Future<Output = eyre::Result<T>>,
    {
        let operation = &operation;
        let runs = self.hosts().iter().map(|host| async move {
            let result = match self.connect(host).await {
                Ok(client) => operation(host.clone(), client).await,
                Err(error) => Err(error),
            };
            (host.clone(), result)
        });

        join_all(runs).await
    }

    /// follows every machine's view, reconnecting on its own, until the
    /// receiver is dropped
    pub fn watch(&self) -> mpsc::Receiver<FleetEvent> {
        let (sender, receiver) = mpsc::channel(256);

        for host in self.hosts() {
            let fleet = self.clone();
            let host = host.clone();
            let sender = sender.clone();
            tokio::spawn(async move { fleet.follow(host, sender).await });
        }

        receiver
    }

    async fn follow(self, host: Hostname, events: mpsc::Sender<FleetEvent>) {
        let link = |link| FleetEvent::Link {
            host: host.clone(),
            link,
        };
        let mut failures = 0usize;

        loop {
            let views = match self.connect(&host).await {
                Ok(client) => client.watch().await,
                Err(error) => Err(error),
            };

            match views {
                Ok(mut views) => {
                    failures = 0;
                    if events.send(link(Link::Connected)).await.is_err() {
                        return;
                    }

                    while let Ok(Some(view)) = views.next().await {
                        let event = FleetEvent::View {
                            host: host.clone(),
                            view: Box::new(view),
                        };
                        if events.send(event).await.is_err() {
                            return;
                        }
                    }

                    if events.send(link(Link::Connecting)).await.is_err() {
                        return;
                    }
                }
                Err(error) => {
                    if events.send(link(Link::Unreachable(error.to_string()))).await.is_err() {
                        return;
                    }
                }
            }

            tokio::time::sleep(Self::BACKOFF[failures.min(Self::BACKOFF.len() - 1)]).await;
            failures += 1;
        }
    }
}
