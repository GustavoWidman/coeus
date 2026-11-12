use std::time::Duration;

use tokio::{sync::mpsc, task::JoinHandle, time::sleep};

use crate::{
    client::{AttachmentEvent, Fleet},
    common::model::{Hostname, JobId, JobSelector},
    ui::{journal, top::Retarget},
};

/// readable lines of a followed job's log
pub struct Batch {
    pub host: Hostname,
    pub job: JobId,
    pub lines: Vec<String>,
    pub ended: bool,
}

/// streams the log of at most one job at a time, resuming where it left
/// off when the connection drops
pub struct Follower {
    fleet: Fleet,
    task: Option<JoinHandle<()>>,
    batches: mpsc::UnboundedSender<Batch>,
}

impl Follower {
    const BACKOFF: [Duration; 4] = [
        Duration::from_secs(1),
        Duration::from_secs(2),
        Duration::from_secs(5),
        Duration::from_secs(10),
    ];

    pub fn new(fleet: Fleet) -> (Self, mpsc::UnboundedReceiver<Batch>) {
        let (batches, receiver) = mpsc::unbounded_channel();
        let follower = Self {
            fleet,
            task: None,
            batches,
        };
        (follower, receiver)
    }

    pub fn retarget(&mut self, retarget: Retarget) {
        if let Some(task) = self.task.take() {
            task.abort();
        }

        if let Retarget::Follow { host, job } = retarget {
            let (fleet, batches) = (self.fleet.clone(), self.batches.clone());
            self.task = Some(tokio::spawn(follow(fleet, host, job, batches)));
        }
    }
}

impl Drop for Follower {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

async fn follow(fleet: Fleet, host: Hostname, job: JobId, batches: mpsc::UnboundedSender<Batch>) {
    let batch = |lines, ended| Batch {
        host: host.clone(),
        job,
        lines,
        ended,
    };
    let mut from = 0;
    let mut failures = 0usize;

    loop {
        let attachment = match fleet.connect(&host).await {
            Ok(client) => client.attach(JobSelector::Id(job), from, None).await,
            Err(error) => Err(error),
        };

        if let Ok(mut attachment) = attachment {
            while let Ok(Some(event)) = attachment.next().await {
                let sent = match event {
                    AttachmentEvent::Started(_) | AttachmentEvent::Screen(_) => Ok(()),
                    AttachmentEvent::Frames { frames, end } => {
                        failures = 0;
                        from = end;
                        let lines = frames.iter().flat_map(journal::frame).collect();
                        batches.send(batch(lines, false))
                    }
                    AttachmentEvent::Ended { .. } => {
                        let _ = batches.send(batch(Vec::new(), true));
                        return;
                    }
                };
                if sent.is_err() {
                    return;
                }
            }
        }

        sleep(Follower::BACKOFF[failures.min(Follower::BACKOFF.len() - 1)]).await;
        failures += 1;
    }
}
