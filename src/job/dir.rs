use std::{
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};

use eyre::{Result, WrapErr, eyre};
use serde::{Serialize, de::DeserializeOwned};

use super::{JobLog, JobSpec, StopRequest};
use crate::common::model::{JobId, JobState, JobSummary, JobView};

/// one job's directory, the only record of the job:
///
/// - `spec.json`, written once at admission
/// - `log`, appended by the job's single writer
/// - `stop`, coeusd's request for the runner to stop
/// - `summary.json`, written with the terminal event, so history never
///   rereads finished logs
/// - `result`, a link to the built system that keeps it from being collected
#[derive(Debug, Clone)]
pub struct JobDir {
    id: JobId,
    path: PathBuf,
}

impl JobDir {
    pub(super) fn new(id: JobId, path: PathBuf) -> Self {
        Self { id, path }
    }

    /// the job directory at `path`, which is named by the job's number
    pub fn open(path: &Path) -> Result<Self> {
        let id = path
            .file_name()
            .and_then(|name| name.to_str()?.parse().ok())
            .ok_or_else(|| eyre!("not a job directory: {}", path.display()))?;
        Ok(Self::new(JobId(id), path.to_owned()))
    }

    pub const fn id(&self) -> JobId {
        self.id
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn log(&self) -> JobLog {
        JobLog::new(self.path.join("log"))
    }

    /// where the runner points the out-link of the built system
    pub fn result(&self) -> PathBuf {
        self.path.join("result")
    }

    pub fn spec(&self) -> Result<JobSpec> {
        read_json(&self.path.join("spec.json"))
            .wrap_err_with(|| format!("job {} has no readable spec", self.id))
    }

    pub(super) fn write_spec(&self, spec: &JobSpec) -> Result<()> {
        write_json(&self.path.join("spec.json"), spec)
    }

    pub fn stop_request(&self) -> Result<Option<StopRequest>> {
        optional(read_json(&self.path.join("stop")))
    }

    pub fn request_stop(&self, request: StopRequest) -> Result<()> {
        write_json(&self.path.join("stop"), &request)
    }

    pub fn summary(&self) -> Result<Option<JobSummary>> {
        optional(read_json(&self.path.join("summary.json")))
    }

    pub fn write_summary(&self, summary: &JobSummary) -> Result<()> {
        write_json(&self.path.join("summary.json"), summary)
    }

    /// the job as its spec describes it, before any runner touched it
    pub fn queued(&self) -> Result<JobView> {
        let spec = self.spec()?;
        Ok(JobView::queued(JobSummary {
            id: spec.id,
            order: spec.order.id,
            source: spec.order.source,
            action: spec.order.action,
            origin: spec.order.origin,
            state: JobState::Queued,
            ordered_at: spec.ordered_at,
            started_at: None,
            finished_at: None,
            toplevel: None,
            attempt: 0,
        }))
    }

    /// the job as its spec and log describe it now
    pub async fn view(&self) -> Result<JobView> {
        if let Some(summary) = self.summary()? {
            return Ok(JobView::queued(summary));
        }

        let mut view = self.queued()?;
        let log = self.log();
        if log.exists() {
            let mut reader = log.reader(0).await?;
            while let Some(event) = reader.next_event().await? {
                view.apply(&event);
            }
        }

        Ok(view)
    }
}

fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T> {
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}

/// writes through a temporary file and a rename, so readers never see half
fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let temporary = path.with_extension("tmp");
    fs::write(&temporary, serde_json::to_vec_pretty(value)?)?;
    fs::rename(&temporary, path)?;
    Ok(())
}

fn optional<T>(result: Result<T>) -> Result<Option<T>> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(error) => match error.downcast_ref::<std::io::Error>() {
            Some(io) if io.kind() == ErrorKind::NotFound => Ok(None),
            _ => Err(error),
        },
    }
}
