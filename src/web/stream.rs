//! the fleet stream a browser reads: a snapshot, then each entity again
//! whenever it changes, as server-sent events. a browser is nothing but its
//! cursor, so a slow one skips the states it was too slow for instead of
//! queueing them.

use std::{fmt, str::FromStr, sync::Arc, time::Duration};

use axum::body::{Body, Bytes};
use eyre::{Report, eyre};
use serde::Serialize;
use tokio::{
    sync::{mpsc, watch},
    time::{Instant, sleep, sleep_until, timeout},
};

use super::{
    fleet::FleetState,
    view::{FleetEvent, SCHEMA, Snapshot, Viewer},
};
use crate::common::model::Hostname;

/// how far a browser has read this process's stream: `<epoch>.<seq>`, sent
/// as each message's id and back as `Last-Event-ID`
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cursor {
    pub epoch: u64,
    pub seq: u64,
}

impl fmt::Display for Cursor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:016x}.{}", self.epoch, self.seq)
    }
}

impl FromStr for Cursor {
    type Err = Report;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let invalid = || eyre!("not a stream cursor: {value}");
        let (epoch, seq) = value.split_once('.').ok_or_else(invalid)?;
        Ok(Self {
            epoch: u64::from_str_radix(epoch, 16).map_err(|_| invalid())?,
            seq: seq.parse().map_err(|_| invalid())?,
        })
    }
}

/// what a snapshot tells a browser besides the fleet
#[derive(Clone, Debug)]
pub struct Masthead {
    pub build: String,
    pub served_by: Hostname,
    pub viewer: Viewer,
}

/// a running build's numbers reach a browser at most this often
pub const PROGRESS: Duration = Duration::from_millis(250);
/// changes this close together leave in one write, so a job ending, which
/// changes a machine, its progress and a deployment, lands at once
pub const TICK: Duration = Duration::from_millis(50);
/// an idle stream says something this often, so nothing between closes it
pub const HEARTBEAT: Duration = Duration::from_secs(15);
/// a browser that takes no bytes for this long is let go; it reconnects
/// with its cursor
pub const UNWRITABLE: Duration = Duration::from_secs(60);

/// the stream for one browser, resuming from `resume` when this process can
pub fn fleet(
    mut fleet: watch::Receiver<Arc<FleetState>>,
    resume: Option<Cursor>,
    masthead: Masthead,
) -> Body {
    // one frame in flight: a browser that stops reading holds up its own
    // writer, never a queue
    let (frames, receiver) = mpsc::channel::<Bytes>(1);
    let state = fleet.borrow_and_update().clone();
    let (mut delivery, first) = Delivery::start(&state, resume, &masthead, Instant::now());

    tokio::spawn(async move {
        if !first.is_empty() && !write(&frames, first).await {
            return;
        }
        loop {
            tokio::select! {
                changed = fleet.changed() => if changed.is_err() {
                    return;
                },
                () = sleep_until(delivery.due.unwrap_or_else(Instant::now)), if delivery.due.is_some() => {}
                () = sleep(HEARTBEAT) => {
                    if !write(&frames, ":\n\n".to_owned()).await {
                        return;
                    }
                    continue;
                }
            }
            sleep(TICK).await;

            let state = fleet.borrow_and_update().clone();
            let frame = delivery.next(&state, Instant::now());
            if !frame.is_empty() && !write(&frames, frame).await {
                return;
            }
        }
    });

    Body::from_stream(futures_util::stream::unfold(
        receiver,
        |mut receiver| async move {
            let frame = receiver.recv().await?;
            Some((Ok::<_, std::convert::Infallible>(frame), receiver))
        },
    ))
}

/// whether the browser took the frame in time
async fn write(frames: &mpsc::Sender<Bytes>, frame: String) -> bool {
    matches!(
        timeout(UNWRITABLE, frames.send(Bytes::from(frame))).await,
        Ok(Ok(()))
    )
}

/// what one browser has been sent. machines, deployments and removals go
/// out as soon as they change; running builds' numbers wait for
/// [PROGRESS] to pass since the last were sent, so each is sent separately
/// up to its own seq.
struct Delivery {
    epoch: u64,
    /// every machine, deployment and removal up to this seq was sent
    sent: u64,
    /// every running build's numbers up to this seq were sent
    progress: u64,
    progress_sent: Option<Instant>,
    /// when numbers held back may go
    due: Option<Instant>,
}

impl Delivery {
    /// where a browser starts, and the frame that brings it up to date
    fn start(
        state: &FleetState,
        resume: Option<Cursor>,
        masthead: &Masthead,
        now: Instant,
    ) -> (Self, String) {
        match resume.filter(|cursor| state.resumes(cursor.epoch, cursor.seq)) {
            Some(cursor) => {
                let mut delivery = Self {
                    epoch: state.epoch,
                    sent: cursor.seq,
                    progress: cursor.seq,
                    progress_sent: None,
                    due: None,
                };
                let frame = delivery.next(state, now);
                (delivery, frame)
            }
            None => {
                let delivery = Self {
                    epoch: state.epoch,
                    sent: state.seq,
                    progress: state.seq,
                    progress_sent: Some(now),
                    due: None,
                };
                let cursor = Cursor {
                    epoch: state.epoch,
                    seq: state.seq,
                };
                let snapshot = FleetEvent::Snapshot(snapshot(state, masthead));
                (delivery, message(cursor, &snapshot))
            }
        }
    }

    /// the frame of what changed since the last, in seq order
    fn next(&mut self, state: &FleetState, now: Instant) -> String {
        let released = self
            .progress_sent
            .is_none_or(|sent| now.duration_since(sent) >= PROGRESS);

        let mut changes: Vec<(u64, FleetEvent)> = Vec::new();
        for card in &state.machines {
            if card.rev > self.sent {
                changes.push((
                    card.rev,
                    FleetEvent::Machine {
                        machine: Box::new(card.value.clone()),
                    },
                ));
            }
        }
        let mut held = false;
        for (host, progress) in &state.progress {
            match &progress.value {
                None if progress.rev > self.sent => changes.push((
                    progress.rev,
                    FleetEvent::ProgressCleared { host: host.clone() },
                )),
                Some(value) if progress.rev > self.progress => match released {
                    true => changes.push((
                        progress.rev,
                        FleetEvent::Progress {
                            progress: value.clone(),
                        },
                    )),
                    false => held = true,
                },
                _ => {}
            }
        }
        for (order, deployment) in &state.deployments {
            if deployment.rev > self.sent {
                let event = match &deployment.value {
                    Some(deployment) => FleetEvent::Deployment {
                        deployment: deployment.clone(),
                    },
                    None => FleetEvent::DeploymentRemoved {
                        order: order.clone(),
                    },
                };
                changes.push((deployment.rev, event));
            }
        }
        changes.sort_unstable_by_key(|(rev, _)| *rev);

        if released {
            if changes
                .iter()
                .any(|(_, event)| matches!(event, FleetEvent::Progress { .. }))
            {
                self.progress_sent = Some(now);
            }
            self.progress = state.seq;
        }
        self.sent = state.seq;
        self.due = held.then(|| self.progress_sent.map_or(now, |sent| sent + PROGRESS));

        // a browser resuming from a message's id must not miss numbers held
        // back from before it, so the id stops short of them
        let resumable = match released {
            true => u64::MAX,
            false => self.progress,
        };
        changes
            .iter()
            .map(|(rev, event)| {
                let cursor = Cursor {
                    epoch: self.epoch,
                    seq: (*rev).min(resumable),
                };
                message(cursor, event)
            })
            .collect()
    }
}

fn snapshot(state: &FleetState, masthead: &Masthead) -> Snapshot {
    let mut deployments: Vec<_> = state
        .deployments
        .values()
        .filter_map(|deployment| deployment.value.clone())
        .collect();
    deployments.sort_by_key(|deployment| std::cmp::Reverse(deployment.ordered_at));

    Snapshot {
        schema: SCHEMA,
        build: masthead.build.clone(),
        served_by: masthead.served_by.clone(),
        viewer: masthead.viewer.clone(),
        machines: state
            .machines
            .iter()
            .map(|card| card.value.clone())
            .collect(),
        progress: state
            .progress
            .values()
            .filter_map(|progress| progress.value.clone())
            .collect(),
        deployments,
    }
}

/// one server-sent event
pub fn message(id: impl fmt::Display, event: &(impl Serialize + Kind)) -> String {
    let data = serde_json::to_string(event).expect("view types always serialize");
    format!("id: {id}\nevent: {}\ndata: {data}\n\n", event.kind())
}

/// the name a message goes by, the same as its `kind` field
pub trait Kind {
    fn kind(&self) -> &'static str;
}

impl Kind for FleetEvent {
    fn kind(&self) -> &'static str {
        match self {
            Self::Snapshot(_) => "snapshot",
            Self::Machine { .. } => "machine",
            Self::Progress { .. } => "progress",
            Self::ProgressCleared { .. } => "progress_cleared",
            Self::Deployment { .. } => "deployment",
            Self::DeploymentRemoved { .. } => "deployment_removed",
        }
    }
}
