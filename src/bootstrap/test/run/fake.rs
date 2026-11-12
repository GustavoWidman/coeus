//! a machine and a forge that live in memory and outlast a run, as the real
//! ones outlast a killed admin; each call is a place a run can be stopped

use std::{
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use eyre::{Result, bail, eyre};
use tokio::sync::Notify;

use crate::{
    bootstrap::{
        Blocker, Bootstrap, SshKey,
        branch::{PullRequest, PullState, body},
        fleet::Reboot,
        remote::{Facts, NixState, Probed, Running, SessionKey, Ssh},
        switch::{
            Door, Ending, Release, Switched,
            first::{self, Stocked},
        },
        world::{Answer, Forge, Machine, NixReady, Reached},
    },
    cli::Console,
    common::model::{
        Action, Attempt, CommitSha, Failure, FdaStatus, JobId, JobState, JobSummary, OrderId,
        Origin, Outcome, Phase, Platform, StorePath, Timestamp,
    },
};

/// stops a run at its `at`-th call into the fake world: that call never
/// returns, and the test drops the run there
#[derive(Clone, Default)]
pub struct Kill(Arc<KillState>);

#[derive(Default)]
struct KillState {
    at: Mutex<Option<usize>>,
    seen: AtomicUsize,
    killed: Notify,
}

impl Kill {
    pub fn at(at: usize) -> Self {
        let kill = Self::default();
        *kill.0.at.lock().unwrap() = Some(at);
        kill
    }

    /// the calls made so far
    pub fn seen(&self) -> usize {
        self.0.seen.load(Ordering::SeqCst)
    }

    pub async fn killed(&self) {
        self.0.killed.notified().await;
    }

    async fn point(&self) {
        let seen = self.0.seen.fetch_add(1, Ordering::SeqCst);
        if *self.0.at.lock().unwrap() == Some(seen) {
            self.0.killed.notify_one();
            std::future::pending::<()>().await;
        }
        tokio::task::yield_now().await;
    }
}

/// the system a stock NixOS runs
const STOCK: &str = "/nix/store/0123456789abcdfghijklmnpqrsvwxyz-nixos-system-nixos-25.11";

/// what the machine is, whatever happens to the runs
pub struct World {
    pub facts: Facts,
    pub host_key: SshKey,
    pub nix: NixState,
    /// root's `authorized_keys` holds the session key
    pub authorized: bool,
    /// coeusd answers on the tailnet: it runs the fleet's configuration
    pub daemon: bool,
    pub jobs: Vec<JobSummary>,
    /// the releases switches were asked for, in order
    pub releases: Vec<Release>,
    pub reboot_required: bool,
    /// how many times a reach went through ssh
    pub ssh: usize,
    /// the next switch fails
    pub breaks: bool,
    /// NixOS refuses to switch live into the fleet's system
    pub refuses: bool,
    /// the system it runs
    pub running: Option<StorePath>,
    /// the system it boots by default
    pub default: Option<StorePath>,
    /// its boot id, one more each boot
    pub boot: u32,
    /// how many times it was rebooted into a boot job's system
    pub reboots: usize,
}

impl World {
    pub fn new(facts: Facts, host_key: SshKey) -> Self {
        Self {
            facts,
            host_key,
            nix: NixState::Installed {
                version: "2.31.2".into(),
            },
            authorized: false,
            daemon: false,
            jobs: Vec::new(),
            releases: Vec::new(),
            reboot_required: true,
            ssh: 0,
            breaks: false,
            refuses: false,
            running: Some(STOCK.parse().unwrap()),
            default: Some(STOCK.parse().unwrap()),
            boot: 1,
            reboots: 0,
        }
    }
}

pub struct FakeMachine {
    pub world: Arc<Mutex<World>>,
    pub kill: Kill,
}

impl FakeMachine {
    fn world(&self) -> std::sync::MutexGuard<'_, World> {
        self.world.lock().unwrap()
    }
}

impl Machine for FakeMachine {
    async fn reach(&self, bootstrap: &Bootstrap, _: &Console) -> Result<Reached> {
        self.kill.point().await;
        if self.world().daemon {
            return Ok(Reached {
                platform: Platform::NixOs,
                detail: format!(
                    "coeusd 0.2.0 answers on {} over the tailnet",
                    bootstrap.host
                ),
                kept: true,
            });
        }
        let state = &bootstrap.state;
        let pinned = Ssh::pinned(state)?;
        let key = self.world().host_key.clone();
        match &pinned {
            Some(pinned) if *pinned != key => {
                return Err(eyre!("the machine presents another key"));
            }
            Some(_) => {}
            None => Ssh::pin(state, &key)?,
        }
        SessionKey::ensure(state)?;
        self.kill.point().await;
        let mut world = self.world();
        world.authorized = true;
        world.ssh += 1;
        let platform = world.facts.platform.into();
        Ok(Reached {
            platform,
            detail: format!("{} · host key pinned", world.facts.describe()),
            kept: pinned.is_some(),
        })
    }

    async fn probed(&self, _: &Bootstrap, _: &Console) -> Result<Probed> {
        self.kill.point().await;
        let world = self.world();
        if world.daemon {
            bail!("the stock user is gone: ssh cannot reach the stock system");
        }
        Ok(Probed {
            facts: world.facts.clone(),
            host_key: world.host_key.clone(),
            nix: world.nix.clone(),
        })
    }

    async fn nix(&self, _: &Bootstrap, _: &Console) -> Result<NixReady> {
        self.kill.point().await;
        let installed = !matches!(self.world().nix, NixState::Installed { .. });
        self.kill.point().await;
        let version = "3.8.2".to_owned();
        let mut world = self.world();
        world.nix = NixState::Installed {
            version: version.clone(),
        };
        world.facts.nix_version = Some(version.clone());
        Ok(NixReady {
            version: Some(version),
            installed,
            moved: Vec::new(),
        })
    }

    async fn switch(
        &self,
        release: &Release,
        bootstrap: &Bootstrap,
        console: &Console,
    ) -> Result<Switched> {
        self.kill.point().await;
        self.world().releases.push(release.clone());
        if self.world().daemon {
            return self.ordered(release, bootstrap.options.retry).await;
        }

        let platform = self.world().facts.platform.into();
        let mut stocked = FakeStock {
            machine: self,
            release,
        };
        let ending = first::converge(
            &mut stocked,
            &bootstrap.host,
            platform,
            bootstrap.options.retry,
            console,
        )
        .await?;
        let job = match ending {
            Ending::Finished(job) => job,
            // the machine comes back on its own
            Ending::Rebooted { job, .. } => {
                self.kill.point().await;
                job
            }
            Ending::Lost { .. } | Ending::DaemonHere => {
                bail!("the fake machine never drops ssh");
            }
        };
        match job.state {
            JobState::Finished(Outcome::Succeeded | Outcome::UpToDate) => Ok(Switched {
                job,
                via: Door::Adopted,
            }),
            _ => Err(Blocker::SwitchFailed { job: Box::new(job) }.into()),
        }
    }

    async fn answer(&self, _: &Switched, _: &Console) -> Result<Answer> {
        self.kill.point().await;
        let world = self.world();
        if !world.daemon {
            bail!("coeusd does not answer");
        }
        Ok(Answer {
            version: "0.2.0".into(),
            address: "100.64.0.7".into(),
            reboot_required: world.reboot_required,
            full_disk_access: FdaStatus::NotApplicable,
        })
    }

    async fn reboot(&self, _: &Answer, _: &Console) -> Result<Reboot> {
        self.kill.point().await;
        self.world().reboot_required = false;
        self.kill.point().await;
        Ok(Reboot::Rebooted {
            took: Duration::from_secs(41),
        })
    }

    async fn cleanup(&self, bootstrap: &Bootstrap, _: &Console) -> Result<bool> {
        self.kill.point().await;
        if SessionKey::existing(&bootstrap.state)?.is_none() {
            return Ok(false);
        }
        self.world().authorized = false;
        self.kill.point().await;
        bootstrap.state.drop_session_key()?;
        Ok(true)
    }
}

impl FakeMachine {
    /// the switch through coeusd, once it answers: the release's job from
    /// history, or one ordered and run to its end
    async fn ordered(&self, release: &Release, retry: bool) -> Result<Switched> {
        let attempt = Attempt::of(&self.world().jobs, &release.nar_hash);
        let order = match attempt {
            Attempt::Succeeded(job) => {
                return Ok(Switched {
                    job,
                    via: Door::Ordered,
                });
            }
            Attempt::Running(job) => job.order,
            Attempt::Fresh(order) => order,
            Attempt::Failed {
                next: Some(next), ..
            } if retry => next,
            Attempt::Failed { last, .. } => {
                return Err(Blocker::SwitchFailed {
                    job: Box::new(last),
                }
                .into());
            }
        };
        let job = self.run_job(release, order, Action::Switch).await;
        match job.state {
            JobState::Finished(Outcome::Succeeded) => Ok(Switched {
                job,
                via: Door::Ordered,
            }),
            _ => Err(Blocker::SwitchFailed { job: Box::new(job) }.into()),
        }
    }

    /// the job of `order`, admitted as `action` unless it was, carried out
    /// by its runner on the machine whether or not anyone follows
    async fn run_job(&self, release: &Release, order: OrderId, action: Action) -> JobSummary {
        let id = {
            let mut world = self.world();
            match world.jobs.iter().position(|job| job.order == order) {
                Some(at) => world.jobs[at].id,
                None => {
                    let id = JobId(world.jobs.len() as u64 + 1);
                    world.jobs.push(job(id, order, action, release));
                    id
                }
            }
        };
        self.kill.point().await;

        let mut world = self.world();
        let at = world
            .jobs
            .iter()
            .position(|job| job.id == id)
            .expect("the job was admitted");
        if let JobState::Finished(_) = world.jobs[at].state {
            return world.jobs[at].clone();
        }
        let toplevel = world.jobs[at].toplevel.clone();
        let outcome = match (world.jobs[at].action, world.refuses, world.breaks) {
            (Action::Boot, ..) => {
                world.default = toplevel;
                Outcome::Succeeded
            }
            (_, true, _) => Outcome::RolledBack(Failure {
                phase: Phase::Activating,
                message: "switch-to-configuration failed with exit status: 1".into(),
                tail: vec![
                    "Checking switch inhibitors... The following inhibitors changed:".into(),
                    "dbus-implementation: dbus -> broker".into(),
                    "Pre-switch check 'switchInhibitors' failed".into(),
                    "Pre-switch checks failed".into(),
                ],
            }),
            (_, _, true) => Outcome::Failed(Failure {
                phase: Phase::Building,
                message: "builder for thoth-0.4 failed with exit code 1".into(),
                tail: vec!["error: linker `cc` not found".into()],
            }),
            _ => {
                world.running = toplevel.clone();
                world.default = toplevel;
                world.daemon = true;
                Outcome::Succeeded
            }
        };
        let job = &mut world.jobs[at];
        job.state = JobState::Finished(outcome);
        job.finished_at = Some(Timestamp::now());
        job.clone()
    }
}

/// the stock machine as the first switch goes through it
struct FakeStock<'a> {
    machine: &'a FakeMachine,
    release: &'a Release,
}

impl Stocked for FakeStock<'_> {
    async fn status(&mut self) -> Result<Attempt> {
        self.machine.kill.point().await;
        Ok(Attempt::of(
            &self.machine.world().jobs,
            &self.release.nar_hash,
        ))
    }

    async fn running(&mut self) -> Result<Running> {
        self.machine.kill.point().await;
        let world = self.machine.world();
        Ok(Running {
            system: world.running.clone(),
            boot: Some(world.boot.to_string()),
        })
    }

    async fn adopt(&mut self, order: OrderId, action: Action) -> Result<Ending> {
        let job = self.machine.run_job(self.release, order, action).await;
        Ok(Ending::Finished(job))
    }

    async fn reboot(&mut self) -> Result<()> {
        self.machine.kill.point().await;
        let mut world = self.machine.world();
        world.running = world.default.clone();
        world.boot += 1;
        world.reboots += 1;
        world.reboot_required = false;
        world.daemon = true;
        Ok(())
    }
}

/// a job admitted for `release`, running
fn job(id: JobId, order: OrderId, action: Action, release: &Release) -> JobSummary {
    let now = Timestamp::now();
    JobSummary {
        id,
        order,
        source: release.source(),
        action,
        origin: Origin::User {
            user: "bootstrap".into(),
            host: "altar".into(),
        },
        state: JobState::Running {
            phase: Phase::Building,
        },
        ordered_at: now,
        started_at: Some(now),
        finished_at: None,
        toplevel: Some(
            format!(
                "/nix/store/{}-nixos-system-nova-26.05",
                "a".repeat(32 - id.0.to_string().len()) + &id.0.to_string()
            )
            .parse::<StorePath>()
            .unwrap(),
        ),
        attempt: 1,
    }
}

/// pull requests, by head branch
#[derive(Default)]
pub struct Pulls {
    pub open: Vec<(String, PullRequest)>,
    /// `gh` fails as if signed out
    pub signed_out: bool,
}

pub struct FakeForge {
    pub pulls: Arc<Mutex<Pulls>>,
    pub kill: Kill,
}

impl FakeForge {
    fn pulls(&self) -> Result<std::sync::MutexGuard<'_, Pulls>> {
        let pulls = self.pulls.lock().unwrap();
        if pulls.signed_out {
            return Err(crate::bootstrap::branch::GhError {
                command: "pr list".into(),
                message: "not logged in to any GitHub hosts".into(),
            }
            .into());
        }
        Ok(pulls)
    }

    fn by_number(&self, number: u64) -> Result<PullRequest> {
        self.pulls()?
            .open
            .iter()
            .map(|(_, pull)| pull.clone())
            .find(|pull| pull.number == number)
            .ok_or_else(|| eyre!("no pull request #{number}"))
    }

    fn change(&self, number: u64, change: impl FnOnce(&mut PullRequest)) -> Result<()> {
        let mut pulls = self.pulls()?;
        let (_, pull) = pulls
            .open
            .iter_mut()
            .find(|(_, pull)| pull.number == number)
            .ok_or_else(|| eyre!("no pull request #{number}"))?;
        change(pull);
        Ok(())
    }
}

/// the branch's head on origin, as the clone saw it pushed: what the forge
/// shows as the head of its open pull requests
fn pushed(clone: &Path, branch: &str) -> Result<CommitSha> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(clone)
        .args(["rev-parse", &format!("refs/remotes/origin/{branch}")])
        .output()?;
    if !output.status.success() {
        bail!("{branch} is not on origin");
    }
    String::from_utf8(output.stdout)?.trim().parse()
}

impl Forge for FakeForge {
    async fn find(&self, clone: &Path, branch: &str) -> Result<Option<PullRequest>> {
        self.kill.point().await;
        let head = pushed(clone, branch).ok();
        let mut pulls = self.pulls()?;
        let listed = pulls.open.iter_mut().filter(|(on, _)| on == branch);
        let listed = listed.map(|(_, pull)| {
            if let Some(head) = head.filter(|_| pull.state == PullState::Open) {
                pull.head = head;
            }
            pull.clone()
        });
        Ok(PullRequest::pick(listed.collect::<Vec<_>>()))
    }

    async fn create_draft(
        &self,
        clone: &Path,
        branch: &str,
        _: &str,
        _: &str,
        body: &str,
    ) -> Result<PullRequest> {
        self.kill.point().await;
        let pull = {
            let mut pulls = self.pulls()?;
            let number = 142 + pulls.open.len() as u64;
            let pull = PullRequest {
                number,
                url: format!("https://github.com/fleet/nix/pull/{number}"),
                draft: true,
                state: PullState::Open,
                body: body.to_owned(),
                head: pushed(clone, branch)?,
            };
            pulls.open.push((branch.to_owned(), pull.clone()));
            pull
        };
        self.kill.point().await;
        Ok(pull)
    }

    async fn edit_body_section(
        &self,
        _: &Path,
        pull: &mut PullRequest,
        section: &str,
    ) -> Result<()> {
        self.kill.point().await;
        let body = body::splice(&pull.body, section);
        if body != pull.body {
            self.change(pull.number, |pull| pull.body = body)?;
            self.kill.point().await;
        }
        *pull = self.by_number(pull.number)?;
        Ok(())
    }

    async fn ready(&self, _: &Path, pull: &mut PullRequest) -> Result<()> {
        self.kill.point().await;
        if pull.draft {
            self.change(pull.number, |pull| pull.draft = false)?;
            self.kill.point().await;
        }
        *pull = self.by_number(pull.number)?;
        Ok(())
    }
}
