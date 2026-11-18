use std::{fmt, future::Future, sync::Arc, time::Duration};

use futures_util::future::join_all;
use tokio::sync::mpsc;

use super::{CoeusClient, Offline, Tailnet, Targets};
use crate::{
    common::model::{Hostname, MachineView, Timestamp},
    config::{ClientSettings, Key},
};

/// the selected machines, reached concurrently
#[derive(Clone)]
pub struct Fleet {
    settings: Arc<ClientSettings>,
    key: Arc<Key>,
    targets: Targets,
    /// what tailscale said once, `None` to ask it before every connection
    tailnet: Option<Arc<Tailnet>>,
}

/// how a watched machine's connection is doing
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Link {
    Connecting,
    Connected,
    Unreachable(Unreach),
}

/// why a machine cannot be reached
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Unreach {
    /// tailscale sees it offline, asleep or away; it catches up once back
    Offline(Offline),
    /// it answered and turned this client away: nothing listens on the
    /// port, it holds another fleet key, or it speaks another protocol
    Refused(String),
    Failed(String),
}

impl Unreach {
    /// the reason behind a failed conversation with a machine
    pub fn of(error: eyre::Report) -> Self {
        error
            .downcast::<Self>()
            .unwrap_or_else(|error| Self::Failed(error.to_string()))
    }
}

/// an offline machine's last sighting is told relative to the moment it is
/// written out, so a reason kept on screen stays current
impl fmt::Display for Unreach {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Offline(offline) => formatter.write_str(&offline.describe(Timestamp::now())),
            Self::Refused(detail) | Self::Failed(detail) => formatter.write_str(detail),
        }
    }
}

impl std::error::Error for Unreach {}

#[derive(Debug)]
pub enum FleetEvent {
    Link {
        host: Hostname,
        link: Link,
    },
    View {
        host: Hostname,
        view: Box<MachineView>,
    },
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
            tailnet: None,
        }
    }

    /// a fleet that takes `tailnet` as tailscale's word on every connection
    #[cfg(test)]
    pub fn seeing(mut self, tailnet: Tailnet) -> Self {
        self.tailnet = Some(Arc::new(tailnet));
        self
    }

    pub fn hosts(&self) -> &[Hostname] {
        self.targets.hosts()
    }

    /// connects to `host`, or gives up on it as soon as tailscale sees it
    /// offline. tailscale is asked alongside, so a machine that answers
    /// waits for no word from it.
    pub async fn connect(&self, host: &Hostname) -> Result<CoeusClient, Unreach> {
        let endpoint = self.settings.endpoint(host);
        let offline = async {
            match &self.tailnet {
                Some(tailnet) => tailnet.offline(&endpoint.host),
                None => Tailnet::read().await?.offline(&endpoint.host),
            }
        };

        tokio::select! {
            connected = CoeusClient::connect(&self.settings, &endpoint, &self.key) => connected,
            Some(offline) = offline => Err(Unreach::Offline(offline)),
        }
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
                Err(unreach) => Err(unreach.into()),
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
                Ok(client) => client.watch().await.map_err(Unreach::of),
                Err(unreach) => Err(unreach),
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
                Err(unreach) => {
                    if events.send(link(Link::Unreachable(unreach))).await.is_err() {
                        return;
                    }
                }
            }

            tokio::time::sleep(Self::BACKOFF[failures.min(Self::BACKOFF.len() - 1)]).await;
            failures += 1;
        }
    }
}
