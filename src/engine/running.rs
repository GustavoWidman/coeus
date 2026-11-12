use std::time::Duration;

use tokio::time::Instant;

use eyre::Result;

use crate::{
    common::model::{JobEvent, JobView, Outcome, Phase},
    job::{JobDir, LogReader},
};

/// the running job, followed through its log
pub(super) struct Running {
    pub dir: JobDir,
    pub view: JobView,
    pub pid: Option<u32>,
    reader: LogReader,
    spawned: Instant,
    /// runners spawned that never took the job
    stillborn: u8,
}

impl Running {
    /// how long a runner may take to take its log before it counts as lost
    const GRACE: Duration = Duration::from_secs(15);
    const STILLBORN: u8 = 3;

    pub async fn open(dir: JobDir) -> Result<Self> {
        let log = dir.log();
        log.touch()?;

        let mut running = Self {
            view: dir.queued()?,
            reader: log.reader(0).await?,
            pid: None,
            spawned: Instant::now(),
            stillborn: 0,
            dir,
        };
        running.follow().await?;
        Ok(running)
    }

    /// applies whatever the runner appended since the last call
    pub async fn follow(&mut self) -> Result<bool> {
        let mut changed = false;

        while let Some(event) = self.reader.next_event().await? {
            if let JobEvent::Started { pid, .. } = event.event {
                self.pid = Some(pid);
            }
            self.view.apply(&event);
            changed = true;
        }

        Ok(changed)
    }

    pub fn outcome(&self) -> Option<&Outcome> {
        match &self.view.summary.state {
            crate::common::model::JobState::Finished(outcome) => Some(outcome),
            _ => None,
        }
    }

    pub fn phase(&self) -> Option<Phase> {
        self.view.summary.state.phase()
    }

    /// the runner is gone without finishing the job
    pub fn lost(&self) -> Result<bool> {
        Ok(self.outcome().is_none()
            && self.spawned.elapsed() > Self::GRACE
            && !self.dir.log().is_held()?)
    }

    /// counts the runner as just started again; false when runners keep
    /// dying before taking the job, which no further attempt will fix
    pub fn respawned(&mut self) -> bool {
        if self.pid.is_none() {
            self.stillborn += 1;
        }
        self.spawned = Instant::now();
        self.pid = None;
        self.stillborn < Self::STILLBORN
    }
}
