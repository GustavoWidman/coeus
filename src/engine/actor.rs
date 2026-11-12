use std::{collections::VecDeque, sync::Arc, time::Duration};

use eyre::Result;
use log::{error, info, warn};
use tokio::{
    sync::{mpsc, oneshot, watch},
    time::{Instant, MissedTickBehavior, interval},
};
use tokio_util::sync::CancellationToken;

use super::{
    Host, SystemFacts,
    follow::{Deployed, Follower},
    lane::Lane,
    notify::Notifier,
    reconcile::{Observed, Plan, Resume},
    running::Running,
    upgrade::UpgradeWatch,
};
use crate::{
    common::model::{
        Action, Admission, CancelReport, DaemonState, FdaStatus, Failure, JobId, JobSelector,
        JobState, JobSummary, MachineView, Order, OrderId, Origin, Outcome, Phase, Placement,
        Rejection, Source, SystemInfo, Timestamp,
    },
    config::DaemonSettings,
    job::{Guard, JobSpec, JobStore, StopRequest, Target},
    nix::FlakeRepo,
};

pub(super) enum Command {
    Deploy {
        order: Order,
        reply: oneshot::Sender<Result<Admission, Rejection>>,
    },
    Cancel {
        job: JobSelector,
        reply: oneshot::Sender<CancelReport>,
    },
}

/// the one task that owns every job's lifecycle on this machine. it reacts
/// to commands and to what runners append to their logs, and publishes the
/// machine's view after every change.
pub(super) struct Actor<H: Host> {
    host: Arc<H>,
    settings: DaemonSettings,
    store: JobStore,
    lane: Lane,
    running: Option<Running>,
    next: Option<JobSummary>,
    /// finished jobs, newest first
    recent: VecDeque<JobSummary>,
    /// recent orders, so a repeated order finds the job it created
    orders: VecDeque<(OrderId, JobId)>,
    next_id: JobId,
    facts: SystemFacts,
    facts_at: Timestamp,
    fda: FdaStatus,
    daemon: DaemonState,
    follower: Option<Follower>,
    notifier: Option<Notifier>,
    upgrade: UpgradeWatch,
    view: watch::Sender<MachineView>,
    restart: CancellationToken,
}

impl<H: Host> Actor<H> {
    const RECENT: usize = MachineView::RECENT;
    const FACTS: Duration = Duration::from_secs(60);
    const FDA: Duration = Duration::from_secs(600);

    /// loads every job from disk and decides what to do with the unfinished
    /// ones before taking any command
    pub async fn load(
        host: Arc<H>,
        settings: DaemonSettings,
        upgrade: UpgradeWatch,
        restart: CancellationToken,
    ) -> Result<(Self, watch::Receiver<MachineView>)> {
        let store = JobStore::open(&settings.state_dir)?;
        let facts = host.facts().await.unwrap_or_default();
        let fda = host.probe_fda().await;

        let mut recent = VecDeque::new();
        let mut orders = VecDeque::new();
        let mut observed = Vec::new();

        for id in store.list()? {
            let dir = store.dir(id);
            let view = match dir.view().await {
                Ok(view) => view,
                Err(error) => {
                    warn!("ignoring unreadable job {id}: {error}");
                    continue;
                }
            };

            orders.push_front((view.summary.order, id));
            match &view.summary.state {
                JobState::Finished(_) => recent.push_front(view.summary),
                state => observed.push(Observed {
                    id,
                    state: state.clone(),
                    held: dir.log().is_held()?,
                    stop: dir.stop_request()?,
                    toplevel: view.summary.toplevel,
                }),
            }
        }

        let repo = FlakeRepo::new(settings.flake.clone());
        let follower = settings.follow.clone().map(|follow| Follower::new(repo, follow));
        let notifier = settings.notify.clone().map(Notifier::new);
        let next_id = store.next_id()?;

        let view = MachineView {
            host: host.info().clone(),
            system: system_info(&facts, Timestamp::now(), fda),
            daemon: DaemonState::Serving,
            running: None,
            next: None,
            recent: Vec::new(),
        };
        let (sender, receiver) = watch::channel(view);

        let mut actor = Self {
            host,
            settings,
            store,
            lane: Lane::Idle,
            running: None,
            next: None,
            recent,
            orders,
            next_id,
            facts,
            facts_at: Timestamp::now(),
            fda,
            daemon: DaemonState::Serving,
            follower,
            notifier,
            upgrade,
            view: sender,
            restart,
        };

        let plan = Plan::reconcile(observed, actor.facts.current.as_ref());
        actor.resume(plan).await?;
        actor.recent.truncate(Self::RECENT);
        actor.publish();

        Ok((actor, receiver))
    }

    pub async fn run(mut self, mut commands: mpsc::Receiver<Command>, shutdown: CancellationToken) {
        let mut tick = interval(Duration::from_millis(100));
        tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
        let mut facts_at = Instant::now();
        let mut fda_at = Instant::now();

        loop {
            tokio::select! {
                biased;
                _ = shutdown.cancelled() => break,
                command = commands.recv() => match command {
                    Some(command) => self.handle(command).await,
                    None => break,
                },
                _ = tick.tick() => {
                    if let Err(error) = self.follow_running().await {
                        error!("failed to follow the running job: {error}");
                    }

                    if facts_at.elapsed() >= Self::FACTS {
                        facts_at = Instant::now();
                        self.refresh_facts().await;
                    }

                    if fda_at.elapsed() >= Self::FDA {
                        fda_at = Instant::now();
                        self.fda = self.host.probe_fda().await;
                    }

                    self.catch_up().await;
                    self.upgrade_when_idle();
                }
            }

            self.publish();
        }
    }

    async fn handle(&mut self, command: Command) {
        match command {
            Command::Deploy { order, reply } => {
                let admission = self.deploy(order).await;
                let _ = reply.send(admission);
            }
            Command::Cancel { job, reply } => {
                let _ = reply.send(self.cancel(job));
            }
        }
    }

    async fn deploy(&mut self, order: Order) -> Result<Admission, Rejection> {
        if let Some((_, job)) = self.orders.iter().find(|(id, _)| *id == order.id) {
            return Ok(Admission {
                job: *job,
                placement: Placement::Duplicate,
            });
        }

        self.admissible(&order).await?;

        let id = self.next_id;
        self.next_id = id.next();

        let spec = self.spec(id, order);
        let now = spec.order.now;
        let dir = match self.store.create(&spec) {
            Ok(dir) => dir,
            Err(error) => {
                error!("failed to record job {id}: {error}");
                return Err(Rejection::Restarting);
            }
        };

        self.orders.push_front((spec.order.id, id));
        self.orders.truncate(self.settings.history.max(Self::RECENT));

        let running_phase = self.running.as_ref().and_then(Running::phase);
        let admitted = self.lane.admit(id, now, running_phase);

        if let Some(replaced) = admitted.replaced {
            self.next = None;
            self.close(replaced, Outcome::Superseded { by: id });
        }

        if let Some(preempted) = admitted.preempt {
            self.stop(preempted, StopRequest::Superseded { by: id });
        }

        match admitted.start {
            Some(job) => self.start(job).await,
            None => self.next = dir.queued().ok().map(|view| view.summary),
        }

        info!("admitted job {id} ({:?})", admitted.placement);
        Ok(Admission {
            job: id,
            placement: admitted.placement,
        })
    }

    async fn admissible(&self, order: &Order) -> Result<(), Rejection> {
        let platform = self.host.info().platform;

        if self.daemon == DaemonState::Restarting {
            return Err(Rejection::Restarting);
        }

        if !self.host.supports(order.action) {
            return Err(Rejection::Unsupported {
                action: order.action,
                platform,
            });
        }

        if order.action != Action::Build && self.fda == FdaStatus::Missing {
            return Err(Rejection::FullDiskAccessMissing {
                launcher: self.settings.launcher.display().to_string(),
            });
        }

        match &order.source {
            Source::Snapshot { nar_hash, .. } => {
                let path = self.sources().join(nar_hash.file_name());
                match path.is_dir() {
                    true => Ok(()),
                    false => Err(Rejection::UnknownSnapshot),
                }
            }
            Source::Generation(number) => {
                let generations = self.host.generations().await.unwrap_or_default();
                match generations.iter().any(|generation| generation.number == *number) {
                    true => Ok(()),
                    false => Err(Rejection::UnknownGeneration(*number)),
                }
            }
            Source::Git { .. } => Ok(()),
        }
    }

    fn spec(&self, id: JobId, order: Order) -> JobSpec {
        JobSpec {
            id,
            order,
            ordered_at: Timestamp::now(),
            target: Target {
                hostname: self.host.info().hostname.clone(),
                platform: self.host.info().platform,
                flake: self.settings.flake.clone(),
                sources: self.sources(),
            },
            ssh_key_file: self.settings.ssh_key_file.clone(),
            guard: self.settings.guard.enable.then(|| Guard {
                timeout: self.settings.guard.timeout,
                daemon_port: self.settings.listen.port(),
                tailscale: self.settings.guard.tailscale,
            }),
        }
    }

    fn sources(&self) -> std::path::PathBuf {
        self.settings.state_dir.join("sources")
    }

    fn cancel(&mut self, selector: JobSelector) -> CancelReport {
        let target = match selector {
            JobSelector::Id(id) => Some(id),
            JobSelector::Current => self.lane.running().or(self.lane.next()),
        };

        let Some(job) = target else {
            return CancelReport::Nothing;
        };

        if self.lane.withdraw(job) {
            self.next = None;
            self.close(job, Outcome::Cancelled);
            return CancelReport::Withdrawn(job);
        }

        match &self.running {
            Some(running) if running.dir.id() == job => match running.phase() {
                Some(phase) if !phase.is_cancellable() => CancelReport::TooLate(job),
                _ => {
                    self.stop(job, StopRequest::Cancel);
                    CancelReport::Stopping(job)
                }
            },
            _ => CancelReport::Nothing,
        }
    }

    /// asks a running job's runner to stop; it honors the request only
    /// before activating
    fn stop(&self, job: JobId, request: StopRequest) {
        let dir = self.store.dir(job);
        if let Err(error) = dir.request_stop(request) {
            error!("failed to ask job {job} to stop: {error}");
            return;
        }

        // the pid comes from the job log: never let it name a process group
        // or every process (0, -1) or init
        let pid = self.running.as_ref().and_then(|running| running.pid);
        let pid = pid
            .and_then(|pid| i32::try_from(pid).ok())
            .filter(|pid| *pid > 1)
            .and_then(rustix::process::Pid::from_raw);

        if let Some(pid) = pid {
            if let Err(error) = rustix::process::kill_process(pid, rustix::process::Signal::TERM) {
                warn!("failed to signal the runner of job {job}: {error}");
            }
        }
    }

    async fn start(&mut self, job: JobId) {
        let dir = self.store.dir(job);

        match Running::open(dir.clone()).await {
            Ok(running) => self.running = Some(running),
            Err(error) => {
                error!("failed to open job {job}: {error}");
                self.fail(job, Phase::Preparing, error.to_string()).await;
                return;
            }
        }

        if let Err(error) = self.host.spawn_runner(&dir).await {
            error!("failed to start a runner for job {job}: {error}");
            self.fail(job, Phase::Preparing, format!("cannot start a runner: {error}"))
                .await;
        }
    }

    /// finishes the running job as failed when no runner could take it
    async fn fail(&mut self, job: JobId, phase: Phase, message: String) {
        self.running = None;
        self.close(job, Outcome::Failed(Failure::new(phase, message)));
        if let Some(next) = self.lane.finish(job) {
            self.next = None;
            Box::pin(self.start(next)).await;
        }
    }

    /// records an outcome for a job no runner holds
    fn close(&mut self, job: JobId, outcome: Outcome) {
        let dir = self.store.dir(job);
        let result = (|| -> Result<JobSummary> {
            let Some(writer) = dir.log().writer()? else {
                eyre::bail!("a runner still holds it");
            };
            let finished = writer.finish(outcome)?;
            let mut view = dir.queued()?;
            view.apply(&finished);
            dir.write_summary(&view.summary)?;
            Ok(view.summary)
        })();

        match result {
            Ok(summary) => self.remember(summary),
            Err(error) => error!("failed to close job {job}: {error}"),
        }
    }

    fn remember(&mut self, summary: JobSummary) {
        if let Some(notifier) = &self.notifier {
            notifier.announce(&self.host.info().hostname, &summary);
        }
        self.recent.push_front(summary);
        self.recent.truncate(Self::RECENT);
    }

    async fn resume(&mut self, plan: Plan) -> Result<()> {
        for (job, outcome) in plan.close {
            info!("closing job {job} left unfinished: {outcome:?}");
            self.close(job, outcome);
        }

        if let Some((job, resume)) = plan.running {
            self.lane.admit(job, false, None);
            match resume {
                Resume::Follow => self.running = Some(Running::open(self.store.dir(job)).await?),
                Resume::Spawn => self.start(job).await,
            }
        }

        if let Some(next) = plan.next {
            self.lane.admit(next, false, None);
            self.next = self.store.dir(next).queued().ok().map(|view| view.summary);
        }

        Ok(())
    }

    async fn follow_running(&mut self) -> Result<()> {
        let Some(running) = &mut self.running else {
            return Ok(());
        };

        running.follow().await?;

        if running.outcome().is_some() {
            return self.complete().await;
        }

        if running.lost()? {
            let job = running.dir.id();
            let toplevel = running.view.summary.toplevel.clone();
            match running.phase() {
                Some(phase) if !phase.is_cancellable() => {
                    let converged = self.facts_now().await.current == toplevel;
                    let outcome = match converged {
                        true => Outcome::Succeeded,
                        false => Outcome::Interrupted,
                    };
                    warn!("the runner of job {job} died while {phase}");
                    self.running = None;
                    self.close(job, outcome);
                    self.after(job).await;
                }
                _ => {
                    warn!("the runner of job {job} died, starting another");
                    running.respawned();
                    let dir = running.dir.clone();
                    if let Err(error) = self.host.spawn_runner(&dir).await {
                        self.fail(job, Phase::Preparing, error.to_string()).await;
                    }
                }
            }
        }

        Ok(())
    }

    /// the running job's runner wrote its outcome
    async fn complete(&mut self) -> Result<()> {
        let Some(running) = self.running.take() else {
            return Ok(());
        };

        let job = running.dir.id();
        if running.dir.summary()?.is_none() {
            running.dir.write_summary(&running.view.summary)?;
        }

        info!("job {job} finished: {:?}", running.outcome());
        self.remember(running.view.summary);
        self.after(job).await;
        Ok(())
    }

    /// cleans up after a job and starts the one waiting behind it
    async fn after(&mut self, job: JobId) {
        if let Err(error) = self.host.release_runner(job).await {
            warn!("failed to release the runner of job {job}: {error}");
        }

        self.refresh_facts().await;

        let finished = |id: JobId| id <= job;
        if let Err(error) = self.store.prune(self.settings.history, finished) {
            warn!("failed to prune old jobs: {error}");
        }

        if let Some(next) = self.lane.finish(job) {
            self.next = None;
            self.start(next).await;
        }
    }

    async fn refresh_facts(&mut self) {
        match self.host.facts().await {
            Ok(facts) => {
                self.facts = facts;
                self.facts_at = Timestamp::now();
            }
            Err(error) => warn!("failed to read the system's state: {error}"),
        }
    }

    async fn facts_now(&mut self) -> SystemFacts {
        self.refresh_facts().await;
        self.facts.clone()
    }

    /// orders a deployment of the followed ref when the machine fell behind
    async fn catch_up(&mut self) {
        let Some(follower) = &mut self.follower else {
            return;
        };

        if self.lane != Lane::Idle || !follower.due() {
            return;
        }

        follower.check().await;
        let deployed = deployed(&self.recent, &self.facts);
        let Some(source) = self.follower.as_ref().and_then(|follower| follower.behind(deployed.as_ref()))
        else {
            return;
        };

        info!("catching up with {source}");
        let order = Order {
            id: OrderId::random(),
            source,
            action: Action::Switch,
            options: Default::default(),
            origin: Origin::Follow,
            now: false,
        };

        if let Err(rejection) = self.deploy(order).await {
            warn!("cannot catch up: {rejection:?}");
        }
    }

    /// exits into a newer coeusd once nothing runs
    fn upgrade_when_idle(&mut self) {
        if self.lane != Lane::Idle || self.daemon == DaemonState::Restarting || !self.upgrade.stale() {
            return;
        }

        info!("a newer coeusd or configuration is installed, restarting into it");
        self.daemon = DaemonState::Restarting;
        self.restart.cancel();
    }

    fn publish(&self) {
        let deployed = deployed(&self.recent, &self.facts);
        let mut system = system_info(&self.facts, self.facts_at, self.fda);
        system.deployed = deployed.as_ref().map(|deployed| deployed.source.clone());
        system.follow = self
            .follower
            .as_ref()
            .map(|follower| follower.state(deployed.as_ref()));

        let view = MachineView {
            host: self.host.info().clone(),
            system,
            daemon: self.daemon,
            running: self.running.as_ref().map(|running| running.view.clone()),
            next: self.next.clone(),
            recent: self.recent.iter().cloned().collect(),
        };

        self.view.send_if_modified(|current| {
            let changed = *current != view;
            if changed {
                *current = view;
            }
            changed
        });
    }
}

/// the newest job that left the machine running what it runs now
fn deployed<'a>(recent: &'a VecDeque<JobSummary>, facts: &SystemFacts) -> Option<Deployed<'a>> {
    recent
        .iter()
        .filter(|job| job.action.activates() && job.toplevel.is_some())
        .find(|job| job.toplevel == facts.current && matches!(job.state, JobState::Finished(Outcome::Succeeded | Outcome::UpToDate)))
        .map(|job| Deployed {
            source: &job.source,
            at: job.finished_at.unwrap_or(job.ordered_at),
        })
}

fn system_info(facts: &SystemFacts, checked_at: Timestamp, fda: FdaStatus) -> SystemInfo {
    SystemInfo {
        current: facts.current.clone(),
        generation: facts.generation,
        pending_boot: facts.pending_boot,
        reboot_required: facts.reboot_required,
        deployed: None,
        follow: None,
        fda,
        checked_at,
    }
}
