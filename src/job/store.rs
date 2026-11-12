use std::{fs, path::PathBuf};

use eyre::Result;
use log::warn;

use super::{JobDir, JobSpec};
use crate::common::model::JobId;

/// the job directories under coeusd's state directory, numbered in order of
/// arrival
#[derive(Debug, Clone)]
pub struct JobStore {
    root: PathBuf,
}

impl JobStore {
    pub fn open(state_dir: &std::path::Path) -> Result<Self> {
        let root = state_dir.join("jobs");
        fs::create_dir_all(&root)?;
        Ok(Self { root })
    }

    pub fn dir(&self, id: JobId) -> JobDir {
        JobDir::new(id, self.root.join(format!("{:06}", id.0)))
    }

    /// every job on disk, oldest first
    pub fn list(&self) -> Result<Vec<JobId>> {
        let mut ids: Vec<JobId> = fs::read_dir(&self.root)?
            .filter_map(|entry| entry.ok()?.file_name().to_str()?.parse().ok().map(JobId))
            .collect();
        ids.sort_unstable();
        Ok(ids)
    }

    /// the id after the newest job on disk
    pub fn next_id(&self) -> Result<JobId> {
        Ok(self.list()?.last().map_or(JobId(1), |last| last.next()))
    }

    /// makes the directory for a newly admitted job
    pub fn create(&self, spec: &JobSpec) -> Result<JobDir> {
        let dir = self.dir(spec.id);
        fs::create_dir(dir.path())?;
        dir.write_spec(spec)?;
        Ok(dir)
    }

    /// removes the oldest finished jobs beyond `keep`
    pub fn prune(&self, keep: usize, finished: impl Fn(JobId) -> bool) -> Result<()> {
        let finished: Vec<JobId> = self.list()?.into_iter().filter(|id| finished(*id)).collect();

        for id in finished.iter().take(finished.len().saturating_sub(keep)) {
            if let Err(error) = fs::remove_dir_all(self.dir(*id).path()) {
                warn!("failed to prune job {id}: {error}");
            }
        }

        Ok(())
    }
}
