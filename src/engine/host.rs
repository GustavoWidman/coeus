use std::future::Future;

use eyre::Result;

use crate::{
    common::model::{Action, FdaStatus, Generation, HostInfo, JobId},
    job::JobDir,
    platform::SystemFacts,
};

/// what the engine needs from the machine it runs on
pub trait Host: Send + Sync + 'static {
    fn info(&self) -> &HostInfo;

    /// whether this platform can carry out `action` at all
    fn supports(&self, action: Action) -> bool;

    fn facts(&self) -> impl Future<Output = Result<SystemFacts>> + Send;

    /// newest first
    fn generations(&self) -> impl Future<Output = Result<Vec<Generation>>> + Send;

    /// starts a runner for the job outside coeusd's own lifetime; a runner
    /// already running for it is not an error
    fn spawn_runner(&self, dir: &JobDir) -> impl Future<Output = Result<()>> + Send;

    /// cleans up whatever the platform kept around for a finished runner
    fn release_runner(&self, id: JobId) -> impl Future<Output = Result<()>> + Send;

    fn probe_fda(&self) -> impl Future<Output = FdaStatus> + Send;

    fn reboot(&self) -> impl Future<Output = Result<()>> + Send;
}
