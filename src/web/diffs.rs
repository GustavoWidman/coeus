//! closure diffs, read from job logs. a machine reports only how many
//! packages a job changes; the list is in the log, written once building is
//! done and never changed after

use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use eyre::{Result, eyre};
use log::debug;
use tokio::{
    sync::{mpsc, watch},
    task::JoinSet,
    time::Instant,
};

use super::{
    fleet::{FleetState, Input},
    source::Source,
    view::{JobStateView, PhaseView},
};
use crate::{
    client::AttachmentEvent,
    common::model::{AttachFrame, ClosureDiff, Hostname, JobEvent, Phase},
};

type Key = (Hostname, u64);

/// the diffs read lately, `None` for a job that never wrote one
#[derive(Default)]
pub struct Diffs {
    entries: Mutex<VecDeque<(Key, Option<Arc<ClosureDiff>>)>>,
    /// how many times the cache was emptied, so a read that began before
    /// keeps what it found to itself
    forgotten: AtomicU64,
}

impl Diffs {
    const CAPACITY: usize = 64;
    /// a job still building writes its diff later, so a reader gives up
    const PATIENCE: Duration = Duration::from_secs(15);

    /// what is known of the job's diff, if it was read
    pub fn cached(&self, host: &Hostname, job: u64) -> Option<Option<Arc<ClosureDiff>>> {
        let mut entries = self.entries.lock().unwrap();
        let at = entries
            .iter()
            .position(|(key, _)| key.0 == *host && key.1 == job)?;
        let entry = entries.remove(at)?;
        let diff = entry.1.clone();
        entries.push_back(entry);
        Some(diff)
    }

    /// empties the cache, for a fleet whose past changed: the simulated one
    /// when its clock moves back
    pub fn forget(&self) {
        let mut entries = self.entries.lock().unwrap();
        self.forgotten.fetch_add(1, Ordering::SeqCst);
        entries.clear();
    }

    /// the job's diff, read from its log unless it was read before
    pub async fn get(
        &self,
        source: &Source,
        host: &Hostname,
        job: u64,
    ) -> Result<Option<Arc<ClosureDiff>>> {
        if let Some(diff) = self.cached(host, job) {
            return Ok(diff);
        }

        let forgotten = self.forgotten.load(Ordering::SeqCst);
        let diff = tokio::time::timeout(Self::PATIENCE, read(source, host, job))
            .await
            .map_err(|_| eyre!("job {job} on {host} wrote no diff yet"))??
            .map(Arc::new);
        let mut entries = self.entries.lock().unwrap();
        if self.forgotten.load(Ordering::SeqCst) == forgotten
            && !entries
                .iter()
                .any(|(key, _)| key.0 == *host && key.1 == job)
        {
            if entries.len() == Self::CAPACITY {
                entries.pop_front();
            }
            entries.push_back(((host.clone(), job), diff.clone()));
        }
        Ok(diff)
    }

    /// reads the diffs of the deployments' jobs as soon as they are
    /// written, telling the aggregator of each, for as long as it runs
    pub async fn follow(
        self: Arc<Self>,
        source: Source,
        mut fleet: watch::Receiver<Arc<FleetState>>,
        inputs: mpsc::Sender<Input>,
    ) {
        const RETRY: Duration = Duration::from_secs(30);
        let mut reading: BTreeSet<Key> = BTreeSet::new();
        let mut failed: BTreeMap<Key, Instant> = BTreeMap::new();
        let mut reads: JoinSet<(Key, bool)> = JoinSet::new();

        loop {
            let wanted = written(&fleet.borrow_and_update());
            failed.retain(|_, at| at.elapsed() < RETRY);
            for key in wanted {
                if self.cached(&key.0, key.1).is_some()
                    || failed.contains_key(&key)
                    || !reading.insert(key.clone())
                {
                    continue;
                }
                let (diffs, source, inputs) = (self.clone(), source.clone(), inputs.clone());
                reads.spawn(async move {
                    let read = diffs.get(&source, &key.0, key.1).await;
                    if let Err(error) = &read {
                        debug!("no diff of job {} on {}: {error:#}", key.1, key.0);
                    }
                    let _ = inputs.send(Input::Diffed).await;
                    (key, read.is_ok())
                });
            }

            tokio::select! {
                changed = fleet.changed() => if changed.is_err() {
                    return;
                },
                Some(Ok((key, read))) = reads.join_next() => {
                    reading.remove(&key);
                    if !read {
                        failed.insert(key, Instant::now());
                    }
                }
            }
        }
    }
}

/// the jobs of the deployments shown that are past building, so their logs
/// hold a diff if they will ever hold one. every deployment counts, not only
/// the newest: a one-machine order placed after a rollout would otherwise
/// hide the rollout's diffs, and each job's log is read once all the same
fn written(state: &FleetState) -> Vec<Key> {
    let rows = state
        .deployments
        .values()
        .filter_map(|deployment| deployment.value.as_ref())
        .flat_map(|deployment| &deployment.rows);

    rows.filter_map(|row| {
        let job = row.job?;
        let card = state
            .machines
            .iter()
            .find(|card| card.value.host == row.host)?;
        let machine = card.value.state.as_ref()?;
        let known = machine
            .running
            .iter()
            .chain(&machine.recent)
            .find(|card| card.id == job)?;
        let past = match known.state {
            JobStateView::Running { phase } => {
                matches!(phase, PhaseView::Activating | PhaseView::Verifying)
            }
            JobStateView::Finished { .. } => true,
            JobStateView::Queued => false,
        };
        past.then(|| (row.host.clone(), job))
    })
    .collect()
}

/// reads the job's log until its diff, or until it is clear none comes
async fn read(source: &Source, host: &Hostname, job: u64) -> Result<Option<ClosureDiff>> {
    let mut feed = source.attach(host, job, 0).await?;
    while let Some(event) = feed.next().await? {
        match event {
            AttachmentEvent::Frames { frames, .. } => {
                for frame in frames {
                    let AttachFrame::Event(stamped) = frame else {
                        continue;
                    };
                    match stamped.event {
                        JobEvent::Diff(diff) => return Ok(Some(diff)),
                        JobEvent::Phase(phase) if phase >= Phase::Activating => return Ok(None),
                        JobEvent::Finished(_) => return Ok(None),
                        _ => {}
                    }
                }
            }
            AttachmentEvent::Ended { .. } => return Ok(None),
            AttachmentEvent::Started(_) | AttachmentEvent::Screen(_) => {}
        }
    }
    Err(eyre!("{host} hung up before job {job}'s log ended"))
}
