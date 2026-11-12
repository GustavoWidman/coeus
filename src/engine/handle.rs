use std::sync::Arc;

use eyre::{Result, eyre};
use tokio::sync::{mpsc, oneshot, watch};

use super::{Host, actor::Command};
use crate::{
    common::model::{
        Admission, CancelReport, Generation, HostInfo, JobSelector, JobSummary, MachineView, Order,
        Pinned, Rejection,
    },
    job::{JobDir, JobStore},
    nix::FlakeRepo,
};

/// the daemon's whole deployment API. cheap to clone: sessions each hold one.
pub struct EngineHandle<H: Host> {
    commands: mpsc::Sender<Command>,
    view: watch::Receiver<MachineView>,
    store: JobStore,
    host: Arc<H>,
    repo: FlakeRepo,
}

impl<H: Host> Clone for EngineHandle<H> {
    fn clone(&self) -> Self {
        Self {
            commands: self.commands.clone(),
            view: self.view.clone(),
            store: self.store.clone(),
            host: self.host.clone(),
            repo: self.repo.clone(),
        }
    }
}

impl<H: Host> EngineHandle<H> {
    pub(super) fn new(
        commands: mpsc::Sender<Command>,
        view: watch::Receiver<MachineView>,
        store: JobStore,
        host: Arc<H>,
        repo: FlakeRepo,
    ) -> Self {
        Self {
            commands,
            view,
            store,
            host,
            repo,
        }
    }

    pub fn host(&self) -> &HostInfo {
        self.host.info()
    }

    pub async fn deploy(&self, order: Order) -> Result<Result<Admission, Rejection>> {
        let (reply, answer) = oneshot::channel();
        self.send(Command::Deploy { order, reply }).await?;
        Ok(answer.await?)
    }

    pub async fn cancel(&self, job: JobSelector) -> Result<CancelReport> {
        let (reply, answer) = oneshot::channel();
        self.send(Command::Cancel { job, reply }).await?;
        Ok(answer.await?)
    }

    /// the machine's view, updated after every change
    pub fn watch(&self) -> watch::Receiver<MachineView> {
        self.view.clone()
    }

    /// the job a selector names right now, with its directory
    pub fn locate(&self, job: JobSelector) -> Option<(JobSummary, JobDir)> {
        let view = self.view.borrow();
        let summary = match job {
            JobSelector::Id(id) => view.job(id).cloned(),
            JobSelector::Current => view.latest().cloned(),
        };
        drop(view);

        let summary = summary.or_else(|| match job {
            JobSelector::Id(id) => self.store.dir(id).summary().ok().flatten(),
            JobSelector::Current => None,
        })?;
        let dir = self.store.dir(summary.id);
        Some((summary, dir))
    }

    /// finished and running jobs, newest first
    pub async fn history(&self, limit: usize) -> Result<Vec<JobSummary>> {
        let mut jobs = Vec::with_capacity(limit);
        for id in self.store.list()?.into_iter().rev().take(limit) {
            if let Ok(view) = self.store.dir(id).view().await {
                jobs.push(view.summary);
            }
        }
        Ok(jobs)
    }

    pub async fn generations(&self) -> Result<Vec<Generation>> {
        self.host.generations().await
    }

    pub async fn resolve(&self, git_ref: &str) -> Result<Pinned> {
        self.repo.resolve(git_ref).await
    }

    pub async fn reboot(&self) -> Result<()> {
        self.host.reboot().await
    }

    async fn send(&self, command: Command) -> Result<()> {
        self.commands
            .send(command)
            .await
            .map_err(|_| eyre!("the engine stopped"))
    }
}
