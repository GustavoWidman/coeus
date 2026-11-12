use std::{fmt, path::Path};

use eyre::{Report, Result};
use log::{info, warn};
use tokio::signal::unix::{Signal, SignalKind, signal};

use super::{
    activation::{self, Activation, Step},
    feed::Feed,
    flake::Recipe,
    nix::{self, Call, Nix},
    process::{Process, Stdout},
};
use crate::{
    common::model::{
        Action, ClosureDiff, Failure, JobEvent, Outcome, Phase, Platform, Source, StorePath,
    },
    job::{JobDir, JobLog, JobSpec, LogWriter, StopRequest},
    platform::{SystemRoot, darwin},
};

/// one job carried out, from its spec to its outcome, by the only process
/// that writes its log
pub struct Runner {
    dir: JobDir,
    spec: JobSpec,
    writer: LogWriter,
    nix: Nix,
    system: SystemRoot,
    phase: Phase,
}

/// what is left once the system is built
enum Next {
    Activate(StorePath),
    Finish(Outcome),
}

impl Runner {
    /// runners a job may go through: each new one means the one before died
    /// with the job unfinished
    const ATTEMPTS: u8 = 3;

    pub async fn run(path: &Path) -> Result<()> {
        // listening from the start keeps a SIGTERM during activation from
        // killing the runner
        let mut stop = signal(SignalKind::terminate())?;

        let dir = JobDir::open(path)?;
        let log = dir.log();
        let Some(writer) = log.writer()? else {
            info!("job {} already has a runner", dir.id());
            return Ok(());
        };

        let history = History::of(&log).await?;
        if history.finished {
            info!("job {} already finished", dir.id());
            return Ok(());
        }

        let spec = dir.spec()?;
        let mut runner = Self {
            nix: Nix::new(&spec),
            dir,
            spec,
            writer,
            system: SystemRoot::default(),
            phase: Phase::Preparing,
        };

        let attempt = history.started.saturating_add(1);
        runner.writer.event(JobEvent::Started {
            attempt,
            pid: std::process::id(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
        })?;

        match runner.outcome(attempt, &mut stop).await {
            Some(outcome) => runner.finish(outcome).await,
            None => {
                info!(
                    "job {} stopped by the system, the next runner resumes it",
                    runner.dir.id()
                );
                Ok(())
            }
        }
    }

    /// how the job ended, or `None` when the system stopped the runner (a
    /// shutdown, a unit stop) without coeusd asking: the job stays unfinished,
    /// so the next runner takes it up as a new attempt
    async fn outcome(&mut self, attempt: u8, stop: &mut Signal) -> Option<Outcome> {
        if attempt > Self::ATTEMPTS {
            return Some(Outcome::Failed(Failure::new(
                Phase::Preparing,
                "the daemon restarted too many times during this job",
            )));
        }
        if let Some(request) = self.stop_request() {
            return Some(request.outcome());
        }

        // dropping the preparation kills the nix process it waits on
        let prepared = tokio::select! {
            prepared = self.prepare() => prepared,
            _ = stop.recv() => return self.stop_request().map(StopRequest::outcome),
        };

        let outcome = match prepared {
            Ok(Next::Activate(toplevel)) => self.activate(toplevel).await,
            Ok(Next::Finish(outcome)) => Ok(outcome),
            Err(error) => Err(error),
        };
        Some(outcome.unwrap_or_else(|error| self.failed(error)))
    }

    /// everything before activation: fetch, evaluate, build and diff
    async fn prepare(&mut self) -> Result<Next> {
        let (platform, action) = (self.spec.target.platform, self.spec.order.action);
        if !platform.supports(action) {
            let failure = Failure::new(Phase::Preparing, format!("{platform} cannot {action}"));
            return Ok(Next::Finish(Outcome::Failed(failure)));
        }

        let (toplevel, drv) = match self.spec.target.recipe(&self.spec.order.source) {
            Recipe::Flake(flake) => {
                let (drv, toplevel) = self.evaluate(&flake).await?;
                (toplevel, Some(drv))
            }
            Recipe::Generation(number) => (self.generation(number).await?, None),
        };

        let current = self.system.current();
        let profile = self.system.profile_system();
        if activation::is_up_to_date(action, &toplevel, current.as_ref(), profile.as_ref()) {
            self.event(JobEvent::Note(
                "the machine already runs this system".into(),
            ))?;
            return Ok(Next::Finish(Outcome::UpToDate));
        }

        if let Some(drv) = drv {
            self.enter(Phase::Building)?;
            let link = self.dir.result();
            self.nix(Call::Build {
                drv: &drv,
                link: &link,
            })
            .await?;
        }

        if let Some(current) = current {
            self.diff(&current, &toplevel).await?;
        }

        Ok(match action {
            Action::Build => Next::Finish(Outcome::Succeeded),
            _ => Next::Activate(toplevel),
        })
    }

    /// fetches the flake and evaluates the system, without building it
    async fn evaluate(&mut self, flake: &str) -> Result<(StorePath, StorePath)> {
        self.enter(Phase::Fetching)?;
        self.nix(Call::Metadata(flake)).await?;

        self.enter(Phase::Evaluating)?;
        let installable = self.spec.target.toplevel(flake);
        let (drv, toplevel) = nix::evaluation(&self.nix(Call::Evaluate(&installable)).await?)?;

        self.event(JobEvent::Evaluated {
            drv: drv.clone(),
            toplevel: toplevel.clone(),
        })?;
        Ok((drv, toplevel))
    }

    /// the system of an existing generation, built long ago
    async fn generation(&mut self, number: u32) -> Result<StorePath> {
        let toplevel = self.system.generation_system(number)?;
        let info = self.nix(Call::PathInfo(&toplevel)).await?;
        // a path copied in without its deriver still names the system
        let drv = nix::deriver(&info).unwrap_or_else(|| toplevel.clone());

        self.event(JobEvent::Evaluated {
            drv,
            toplevel: toplevel.clone(),
        })?;
        Ok(toplevel)
    }

    /// a diff only informs, so one nix cannot produce is noted, not fatal
    async fn diff(&mut self, current: &StorePath, toplevel: &StorePath) -> Result<()> {
        let call = Call::DiffClosures {
            from: current,
            to: toplevel,
        };

        match self.nix(call).await {
            Ok(stdout) => {
                let diff = ClosureDiff::parse(&String::from_utf8_lossy(&stdout));
                self.event(JobEvent::Diff(diff))
            }
            Err(error) => {
                let failure = self.failure(error);
                self.event(JobEvent::Note(format!(
                    "no closure diff: {}",
                    failure.message
                )))
            }
        }
    }

    async fn activate(&mut self, toplevel: StorePath) -> Result<Outcome> {
        let (platform, action) = (self.spec.target.platform, self.spec.order.action);

        let undo = Activation::undo(platform, action, &self.system);
        let activation = Activation {
            platform,
            action,
            toplevel,
            generation: match self.spec.order.source {
                Source::Generation(number) => Some(number),
                _ => None,
            },
        };

        self.enter(Phase::Activating)?;
        for step in activation.steps() {
            self.exec(&step)
                .await
                .map_err(|error| self.without_full_disk_access(platform, error))?;
        }

        let Some(guard) = self.spec.guard.clone().filter(|_| action.activates()) else {
            return Ok(Outcome::Succeeded);
        };

        self.enter(Phase::Verifying)?;
        let Err(problem) = guard.verify().await else {
            return Ok(Outcome::Succeeded);
        };

        let Some(undo) = undo else {
            let message = format!("{problem}, and there is no earlier system to go back to");
            return Ok(Outcome::Failed(Failure::new(Phase::Verifying, message)));
        };

        self.event(JobEvent::Note(format!(
            "{problem}: going back to {}",
            undo.describe()
        )))?;
        for step in undo.steps() {
            self.exec(&step).await?;
        }

        Ok(Outcome::RolledBack(Failure::new(Phase::Verifying, problem)))
    }

    /// runs one nix call with its log in the job log, and returns its stdout
    async fn nix(&mut self, call: Call<'_>) -> Result<Vec<u8>> {
        let process = Process::spawn(self.nix.command(&call))?;
        let mut feed = Feed::new(&mut self.writer);
        let exit = process.finish(&mut feed, Stdout::Capture).await?;

        if !exit.status.success() {
            return Err(Failed(feed.fold().failure(self.phase)).into());
        }
        Ok(exit.stdout)
    }

    /// runs one activation step with all its output in the job log
    async fn exec(&mut self, step: &Step) -> Result<()> {
        let process = Process::spawn(step.command())?;
        let mut feed = Feed::new(&mut self.writer);
        let exit = process.finish(&mut feed, Stdout::Log).await?;

        if !exit.status.success() {
            return Err(Failed(Failure {
                phase: self.phase,
                message: format!("{} failed with {}", step.program, exit.status),
                tail: feed.fold().tail().map(str::to_owned).collect(),
            })
            .into());
        }
        Ok(())
    }

    fn enter(&mut self, phase: Phase) -> Result<()> {
        self.phase = phase;
        self.event(JobEvent::Phase(phase))
    }

    fn event(&mut self, event: JobEvent) -> Result<()> {
        self.writer.event(event).map(drop)
    }

    fn stop_request(&self) -> Option<StopRequest> {
        self.dir
            .stop_request()
            .inspect_err(|error| warn!("unreadable stop request: {error:#}"))
            .ok()
            .flatten()
    }

    /// names the missing grant when a darwin activation fails without it,
    /// since nix-darwin then aborts on user changes
    fn without_full_disk_access(&self, platform: Platform, error: Report) -> Report {
        if platform != Platform::Darwin || darwin::has_full_disk_access() {
            return error;
        }

        let mut failure = self.failure(error);
        failure.message = format!(
            "{}; {}",
            failure.message,
            darwin::full_disk_access_missing(&darwin::launcher())
        );
        Failed(failure).into()
    }

    fn failure(&self, error: Report) -> Failure {
        match error.downcast::<Failed>() {
            Ok(Failed(failure)) => failure,
            Err(error) => Failure::new(self.phase, format!("{error:#}")),
        }
    }

    fn failed(&self, error: Report) -> Outcome {
        Outcome::Failed(self.failure(error))
    }

    /// ends the log and leaves the summary history reads instead of it
    async fn finish(self, outcome: Outcome) -> Result<()> {
        info!("job {} finished: {outcome:?}", self.dir.id());
        self.writer.finish(outcome)?;

        let view = self.dir.view().await?;
        self.dir.write_summary(&view.summary)
    }
}

/// what earlier runners left in a job's log
#[derive(Debug, Default, PartialEq, Eq)]
pub struct History {
    pub started: u8,
    pub finished: bool,
}

impl History {
    pub async fn of(log: &JobLog) -> Result<Self> {
        let mut history = Self::default();
        let mut reader = log.reader(0).await?;

        while let Some(stamped) = reader.next_event().await? {
            match stamped.event {
                JobEvent::Started { .. } => history.started = history.started.saturating_add(1),
                JobEvent::Finished(_) => history.finished = true,
                _ => {}
            }
        }

        Ok(history)
    }
}

/// a step that failed with an explanation for people
#[derive(Debug)]
struct Failed(Failure);

impl fmt::Display for Failed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0.message)
    }
}

impl std::error::Error for Failed {}
