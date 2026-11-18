//! six machines and a rollout, simulated: lab is building job 61 of a
//! deployment whose canary went first, desktop-nixos failed it, laptop-mac
//! sleeps through it and wakes to catch up, and then it all starts over.
//! one seeded generator draws every number, so a moment of the timeline is
//! the same however it is reached.

use sha2::{Digest, Sha256};

use super::{
    log::Log,
    story::{HISTORY, MACHINES, PACKAGES, diffs, printed},
};
use crate::common::model::{
    Action, ClosureDiff, CommitSha, DaemonState, Failure, FdaStatus, Follow, HostInfo, Hostname,
    JobEvent, JobId, JobState, JobSummary, JobView, MachineView, OrderId, Origin, Outcome,
    PackageChange, Phase, Pinned, Platform, Progress, Source, StorePath, SystemInfo, Tally,
    Timestamp,
};

/// the simulation advances in steps this long, in milliseconds
pub const STEP: i64 = 50;
const SECOND: f64 = 1_000.0;
const MINUTE: f64 = 60_000.0;
const MIB: f64 = 1_048_576.0;

pub const HOSTS: [&str; 6] = [
    "altar",
    "desktop-nixos",
    "lab",
    "laptop-mac",
    "oracle-1",
    "oracle-2",
];
const ALTAR: usize = 0;
const DESKTOP: usize = 1;
const LAB: usize = 2;
const LAPTOP: usize = 3;
const ORACLE_1: usize = 4;
const ORACLE_2: usize = 5;

const PHASES: [Phase; 6] = [
    Phase::Preparing,
    Phase::Fetching,
    Phase::Evaluating,
    Phase::Building,
    Phase::Activating,
    Phase::Verifying,
];
const BUILDING: usize = 3;
const ACTIVATING: usize = 4;
const VERIFYING: usize = 5;

/// the commit the deployment rolls out
const RELEASE: &str = "2cbe6da";
const REPOSITORY: &str = "GustavoWidman/nix";

/// mulberry32, the prototype's generator
#[derive(Clone, Debug)]
struct Rng(u32);

impl Rng {
    fn next(&mut self) -> f64 {
        self.0 = self.0.wrapping_add(0x6D2B_79F5);
        let a = self.0;
        let mut t = (a ^ (a >> 15)).wrapping_mul(1 | a);
        t = t.wrapping_add((t ^ (t >> 7)).wrapping_mul(61 | t)) ^ t;
        f64::from(t ^ (t >> 14)) / 4_294_967_296.0
    }

    /// a whole number from `low` to `high`, both included
    fn between(&mut self, low: u32, high: u32) -> u32 {
        low + (self.next() * f64::from(high - low + 1)) as u32
    }

    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[(self.next() * items.len() as f64) as usize]
    }

    /// the hash part of a store path
    fn hash(&mut self) -> String {
        const DIGITS: &[u8] = b"0123456789abcdfghijklmnpqrsvwxyz";
        (0..32).map(|_| char::from(*self.pick(DIGITS))).collect()
    }
}

/// a commit named by its short hash; the rest is made up, the same each time
fn commit(short: &str) -> CommitSha {
    let digest = Sha256::digest(short.as_bytes());
    let rest: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    format!("{short}{}", &rest[..40 - short.len()])
        .parse()
        .expect("forty hex digits")
}

/// an order named by what placed it
fn order(label: &str) -> OrderId {
    let digest = Sha256::digest(label.as_bytes());
    OrderId(u64::from_be_bytes(digest[..8].try_into().unwrap()))
}

/// `main@2cbe6da`, `snapshot 3f9a1c0` or `generation 1361`
fn source(label: &str) -> Source {
    if let Some((git_ref, short)) = label.split_once('@') {
        return Source::Git {
            rev: commit(short),
            git_ref: Some(git_ref.to_owned()),
        };
    }
    if let Some(number) = label.strip_prefix("generation ") {
        return Source::Generation(number.parse().expect("a generation number"));
    }
    let label = label.strip_prefix("snapshot ").expect("a snapshot");
    let digest = Sha256::digest(label.as_bytes());
    let encoded = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, digest);
    Source::Snapshot {
        nar_hash: format!("sha256-{encoded}").parse().expect("an SRI hash"),
        label: label.to_owned(),
    }
}

/// `actions · deploy #1184`, `catch-up` or `gustavo@altar`
fn origin(label: &str) -> Origin {
    if let Some(run) = label.strip_prefix("actions · deploy #") {
        return Origin::GitHub {
            repository: REPOSITORY.to_owned(),
            workflow: "deploy".to_owned(),
            run_id: run.parse().expect("a run number"),
            attempt: 1,
        };
    }
    if label == "catch-up" {
        return Origin::Follow;
    }
    let (user, host) = label.split_once('@').expect("user@host");
    Origin::User {
        user: user.to_owned(),
        host: host.to_owned(),
    }
}

/// the short hash a store path's name ends in
fn short(source: &Source, generation: u32) -> String {
    match source {
        Source::Git { rev, .. } => rev.short(),
        Source::Snapshot { label, .. } => label.clone(),
        Source::Generation(_) => format!("g{generation}"),
    }
}

/// how much a job builds and downloads, and how fast, per second
#[derive(Clone, Copy, Debug)]
struct Work {
    builds: (u32, u32),
    downloads: (u32, u32),
    bytes: f64,
    build_rate: f64,
    download_rate: f64,
}

/// seconds each phase takes; building takes as long as its work
type Plan = [f64; 6];

const fn plan(
    preparing: f64,
    fetching: f64,
    evaluating: f64,
    activating: f64,
    verifying: f64,
) -> Plan {
    [preparing, fetching, evaluating, 0.0, activating, verifying]
}

#[derive(Clone, Debug)]
struct Job {
    summary: JobSummary,
    /// seconds a queued job waits before its runner takes it
    wait: f64,
    plan: Plan,
    /// index into [PHASES] while running
    phase: usize,
    /// seconds spent in the phase
    elapsed: f64,
    phase_started_at: Option<Timestamp>,
    work: Option<Work>,
    progress: Progress,
    /// derivations building, with the activity each builds under
    active: Vec<(String, u64)>,
    built: f64,
    downloaded: f64,
    /// seconds until the oldest derivation building finishes
    rotate: f64,
    diff: Option<ClosureDiff>,
    changes: Option<u32>,
    /// part of the rollout
    rollout: bool,
    log: Log,
}

impl Job {
    fn new(summary: JobSummary, plan: Plan, work: Option<Work>) -> Self {
        Self {
            summary,
            wait: 1.2,
            plan,
            phase: 0,
            elapsed: 0.0,
            phase_started_at: None,
            work,
            progress: Progress::default(),
            active: Vec::new(),
            built: 0.0,
            downloaded: 0.0,
            rotate: 1.0,
            diff: None,
            changes: None,
            rollout: false,
            log: Log::default(),
        }
    }

    fn view(&self) -> JobView {
        JobView {
            summary: self.summary.clone(),
            progress: self.progress.clone(),
            phase_started_at: self.phase_started_at,
            changes: self.changes,
        }
    }
}

#[derive(Clone, Debug)]
struct Machine {
    info: HostInfo,
    reachable: bool,
    last_seen: i64,
    generation: u32,
    current: StorePath,
    deployed: Source,
    pending_boot: bool,
    reboot_required: bool,
    paused: bool,
    /// the running job, or the next one while its runner starts
    job: Option<Job>,
    next: Option<Job>,
    /// newest first
    recent: Vec<Job>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Verdict {
    Waiting,
    Running,
    Done,
    Failed,
    Unreachable,
    CatchingUp,
    CaughtUp,
}

/// what the `coeus deploy` running the rollout knows
#[derive(Clone, Debug)]
struct Rollout {
    label: &'static str,
    source: Source,
    origin: Origin,
    waves: Vec<(Vec<usize>, bool)>,
    wave: usize,
    verdicts: [Verdict; 6],
    diffs: [Vec<PackageChange>; 6],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Timer {
    StartWave,
    Wake,
    Restart,
}

#[derive(Clone, Debug)]
pub struct World {
    rng: Rng,
    base: i64,
    pub now: i64,
    machines: Vec<Machine>,
    timers: Vec<(i64, Timer)>,
    rollout: Rollout,
    /// when the story ended and the fleet would start over
    pub ended: Option<i64>,
}

/// the jobs each machine of the last wave and the catch-up runs
fn last_wave(host: usize) -> (u64, Plan, Work) {
    match host {
        ALTAR => (
            43,
            plan(1.0, 1.8, 7.0, 2.4, 1.8),
            Work {
                builds: (0, 14),
                downloads: (0, 162),
                bytes: 214.0 * MIB,
                build_rate: 1.6,
                download_rate: 22.0,
            },
        ),
        _ => (
            37,
            plan(1.4, 2.6, 8.0, 2.8, 2.0),
            Work {
                builds: (0, 9),
                downloads: (0, 248),
                bytes: 391.0 * MIB,
                build_rate: 1.2,
                download_rate: 28.0,
            },
        ),
    }
}

/// one row of [HISTORY]
struct Past {
    host: usize,
    id: u64,
    action: Action,
    source: &'static str,
    origin: &'static str,
    outcome: Outcome,
    /// minutes ago it finished
    ago: f64,
    /// seconds it ran
    took: f64,
}

fn history() -> impl Iterator<Item = Past> {
    HISTORY.lines().filter(|line| !line.is_empty()).map(|line| {
        let fields: Vec<&'static str> = line.split('|').map(str::trim).collect();
        let [host, id, action, source, origin, outcome, ago, took] = fields[..] else {
            panic!("a history row has eight fields: {line}");
        };
        Past {
            host: HOSTS.iter().position(|name| *name == host).expect("a host"),
            id: id.parse().expect("a job id"),
            action: match action {
                "switch" => Action::Switch,
                "boot" => Action::Boot,
                "test" => Action::Test,
                _ => Action::Build,
            },
            source,
            origin,
            outcome: outcome_named(outcome),
            ago: ago.parse().expect("minutes"),
            took: took.parse().expect("seconds"),
        }
    })
}

/// `succeeded`, `superseded 60`, `failed building: <message>`, ...
fn outcome_named(text: &str) -> Outcome {
    let (kind, rest) = text.split_once(' ').unwrap_or((text, ""));
    let failure = || {
        let (phase, message) = rest.split_once(": ").expect("phase: message");
        let phase = PHASES
            .into_iter()
            .find(|known| known.as_str() == phase)
            .expect("a phase");
        Failure::new(phase, message)
    };
    match kind {
        "succeeded" => Outcome::Succeeded,
        "up_to_date" => Outcome::UpToDate,
        "cancelled" => Outcome::Cancelled,
        "interrupted" => Outcome::Interrupted,
        "superseded" => Outcome::Superseded {
            by: JobId(rest.parse().expect("a job id")),
        },
        "failed" => Outcome::Failed(failure()),
        "rolled_back" => Outcome::RolledBack(failure()),
        _ => panic!("no outcome {text}"),
    }
}

const D1184: &str = "actions · deploy #1184";

impl World {
    /// the fleet as the story starts at `base`
    pub fn seed(base: i64) -> Self {
        let ago = |minutes: f64| base - (minutes * MINUTE) as i64;
        let mut world = Self {
            rng: Rng(20_261_117),
            base,
            now: base,
            machines: Vec::new(),
            timers: Vec::new(),
            rollout: Rollout {
                label: D1184,
                source: source(&format!("main@{RELEASE}")),
                origin: origin(D1184),
                waves: vec![
                    (vec![ORACLE_1], true),
                    (vec![LAB, ORACLE_2, DESKTOP], false),
                    (vec![ALTAR, LAPTOP], false),
                ],
                wave: 1,
                verdicts: [
                    Verdict::Waiting,
                    Verdict::Failed,
                    Verdict::Running,
                    Verdict::Waiting,
                    Verdict::Done,
                    Verdict::Done,
                ],
                diffs: diffs(),
            },
            ended: None,
        };

        for (at, (platform, system, generation, deployed)) in MACHINES.into_iter().enumerate() {
            let deployed = source(deployed);
            let machine = Machine {
                info: HostInfo {
                    hostname: HOSTS[at].parse().expect("a hostname"),
                    platform,
                    system: system.to_owned(),
                    version: if at == LAPTOP { "0.4.0" } else { "0.4.1" }.to_owned(),
                },
                reachable: at != LAPTOP,
                last_seen: if at == LAPTOP { ago(192.0) } else { base },
                generation,
                current: world.store(at, &deployed, generation),
                deployed,
                pending_boot: at == ORACLE_2,
                reboot_required: at == ORACLE_1,
                paused: false,
                job: None,
                next: None,
                recent: Vec::new(),
            };
            world.machines.push(machine);
        }
        for past in history() {
            let job = world.history(&past, ago(past.ago));
            world.machines[past.host].recent.push(job);
        }
        world.align(ago(6.7), ago(4.6));
        world.building(ago(4.6) + 120 * LAB as i64, ago(4.4), ago(0.7));
        world
    }

    /// a finished job and its log, which a seeded generator of its own
    /// writes, so history never moves the fleet's generator
    fn history(&self, past: &Past, finished: i64) -> Job {
        let host = past.host;
        let started = finished - (past.took * SECOND) as i64;
        let origin = origin(past.origin);
        let source = source(past.source);
        let label = match &origin {
            Origin::GitHub { .. } => past.origin.to_owned(),
            _ => format!("{} #{}", HOSTS[host], past.id),
        };
        let summary = JobSummary {
            id: JobId(past.id),
            order: order(&label),
            source,
            action: past.action,
            origin,
            state: JobState::Finished(past.outcome.clone()),
            ordered_at: Timestamp(started - 4_000),
            started_at: Some(Timestamp(started)),
            finished_at: Some(Timestamp(finished)),
            toplevel: None,
            attempt: 1,
        };
        let diff =
            (past.origin == self.rollout.label && past.outcome == Outcome::Succeeded).then(|| {
                ClosureDiff {
                    changes: self.rollout.diffs[host].clone(),
                }
            });

        let mut job = Job::new(summary, [0.0; 6], None);
        job.log = written(&job.summary, HOSTS[host], diff.as_ref());
        job.changes = diff.as_ref().map(|diff| diff.changes.len() as u32);
        job.diff = diff;
        job
    }

    /// the jobs of one order were ordered together: the rollout's first two
    /// waves pause between them, every other order is one wave
    fn align(&mut self, canary: i64, second: i64) {
        let mut first: std::collections::BTreeMap<u64, Timestamp> = Default::default();
        for job in self.machines.iter().flat_map(|machine| &machine.recent) {
            let at = first
                .entry(job.summary.order.0)
                .or_insert(job.summary.ordered_at);
            *at = (*at).min(job.summary.ordered_at);
        }
        let release = order(self.rollout.label);
        for (host, machine) in self.machines.iter_mut().enumerate() {
            for job in &mut machine.recent {
                job.summary.ordered_at = match job.summary.order == release {
                    true if host == ORACLE_1 => Timestamp(canary),
                    true => Timestamp(second + 120 * host as i64),
                    false => first[&job.summary.order.0],
                };
            }
        }
    }

    /// lab, three phases into job 61 and building, with job 62 queued
    fn building(&mut self, ordered: i64, started: i64, queued: i64) {
        let work = Work {
            builds: (112, 388),
            downloads: (640, 1530),
            bytes: 598.0 * MIB,
            build_rate: 9.5,
            download_rate: 70.0,
        };
        let mut job = self.ordered(
            LAB,
            61,
            self.rollout.source.clone(),
            self.rollout.origin.clone(),
            plan(1.3, 4.1, 38.0, 3.4, 2.8),
            work,
        );
        job.rollout = true;
        job.diff = Some(ClosureDiff {
            changes: self.rollout.diffs[LAB].clone(),
        });
        job.summary.ordered_at = Timestamp(ordered);
        job.summary.started_at = Some(Timestamp(started));
        job.summary.state = JobState::Running {
            phase: Phase::Preparing,
        };
        job.log.event(started, started_event());

        let mut at = started;
        for (phase, &kind) in PHASES.iter().enumerate().take(BUILDING) {
            job.phase = phase;
            job.log.event(at, JobEvent::Phase(kind));
            self.phase_log(LAB, &mut job, at);
            at += (job.plan[phase] * SECOND) as i64;
        }
        self.evaluated(LAB, &mut job, at);
        job.phase = BUILDING;
        job.summary.state = JobState::Running {
            phase: Phase::Building,
        };
        job.phase_started_at = Some(Timestamp(at));
        job.elapsed = (self.base - at) as f64 / SECOND;
        job.log.event(at, JobEvent::Phase(Phase::Building));
        self.start_work(&mut job);
        scroll(&mut self.rng, &mut job.log, 70, 0.35, 0.6);
        for _ in 0..4 {
            self.add_derivation(&mut job);
        }
        job.progress.builds.running = job.active.len() as u32;
        self.machines[LAB].job = Some(job);

        let mut next = self.ordered(
            LAB,
            62,
            source("snapshot 3f9a1c0"),
            origin("gustavo@altar"),
            plan(0.8, 3.2, 5.5, 2.6, 2.0),
            Work {
                builds: (0, 4),
                downloads: (0, 18),
                bytes: 41.0 * MIB,
                build_rate: 1.2,
                download_rate: 9.0,
            },
        );
        next.summary.ordered_at = Timestamp(queued);
        self.machines[LAB].next = Some(next);
    }

    /// a queued switch, ordered now
    fn ordered(
        &self,
        host: usize,
        id: u64,
        source: Source,
        origin: Origin,
        plan: Plan,
        work: Work,
    ) -> Job {
        let label = match &origin {
            Origin::GitHub { .. } => self.rollout.label.to_owned(),
            _ => format!("{} #{id}", HOSTS[host]),
        };
        let summary = JobSummary {
            id: JobId(id),
            order: order(&label),
            source,
            action: Action::Switch,
            origin,
            state: JobState::Queued,
            ordered_at: Timestamp(self.now),
            started_at: None,
            finished_at: None,
            toplevel: None,
            attempt: 1,
        };
        Job::new(summary, plan, Some(work))
    }

    fn store(&mut self, host: usize, source: &Source, generation: u32) -> StorePath {
        let name = match host {
            ALTAR | LAPTOP => "darwin-system".to_owned(),
            _ => format!("nixos-system-{}", HOSTS[host]),
        };
        let hash = self.rng.hash();
        format!(
            "/nix/store/{hash}-{name}-25.11.20261114.{}",
            short(source, generation)
        )
        .parse()
        .expect("a store path")
    }

    /// moves the fleet [STEP] milliseconds on
    pub fn step(&mut self) {
        self.now += STEP;
        let seconds = STEP as f64 / SECOND;
        for host in 0..self.machines.len() {
            if self.machines[host].reachable {
                self.machines[host].last_seen = self.now;
            }
            let Some(mut job) = self.machines[host].job.take() else {
                continue;
            };
            if job.summary.state == JobState::Queued {
                job.wait -= seconds;
                if job.wait <= 0.0 {
                    self.start(host, &mut job);
                }
                self.machines[host].job = Some(job);
                continue;
            }

            job.elapsed += seconds;
            if job.phase == BUILDING {
                self.build(host, &mut job, seconds);
                self.machines[host].job = Some(job);
            } else if job.elapsed >= job.plan[job.phase] {
                if job.phase == VERIFYING {
                    self.finish(host, job, Outcome::Succeeded);
                } else {
                    let next = job.phase + 1;
                    self.enter(host, &mut job, next);
                    self.machines[host].job = Some(job);
                }
            } else {
                self.machines[host].job = Some(job);
            }
        }

        let mut at = 0;
        while at < self.timers.len() {
            if self.now >= self.timers[at].0 {
                let (_, timer) = self.timers.remove(at);
                self.fire(timer);
            } else {
                at += 1;
            }
        }
    }

    fn later(&mut self, seconds: f64, timer: Timer) {
        self.timers
            .push((self.now + (seconds * SECOND) as i64, timer));
    }

    fn start(&mut self, host: usize, job: &mut Job) {
        job.summary.started_at = Some(Timestamp(self.now));
        job.log.event(self.now, started_event());
        job.phase = 0;
        self.enter(host, job, 0);
    }

    fn enter(&mut self, host: usize, job: &mut Job, phase: usize) {
        if phase == BUILDING {
            self.evaluated(host, job, self.now);
        }
        if phase == ACTIVATING
            && job.phase == BUILDING
            && let Some(diff) = job.diff.clone()
        {
            job.changes = Some(diff.changes.len() as u32);
            job.log.event(self.now, JobEvent::Diff(diff));
        }
        job.phase = phase;
        job.elapsed = 0.0;
        job.phase_started_at = Some(Timestamp(self.now));
        job.summary.state = JobState::Running {
            phase: PHASES[phase],
        };
        job.log.event(self.now, JobEvent::Phase(PHASES[phase]));
        self.phase_log(host, job, self.now);
        if phase == BUILDING {
            self.start_work(job);
        }
    }

    /// evaluation names the system the job builds
    fn evaluated(&mut self, host: usize, job: &mut Job, at: i64) {
        let generation = self
            .machines
            .get(host)
            .map_or(0, |machine| machine.generation);
        let toplevel = self.store(host, &job.summary.source.clone(), generation);
        let drv: StorePath = format!("{toplevel}.drv").parse().expect("a store path");
        job.summary.toplevel = Some(toplevel.clone());
        job.log.event(at, JobEvent::Evaluated { drv, toplevel });
    }

    fn phase_log(&mut self, host: usize, job: &mut Job, at: i64) {
        let log = &mut job.log;
        match PHASES[job.phase] {
            Phase::Preparing => {
                log.event(at, JobEvent::Note("took /var/lib/coeus/job.lock".into()))
            }
            Phase::Fetching => match &job.summary.source {
                Source::Snapshot { label, .. } => log.event(
                    at,
                    JobEvent::Note(format!(
                        "received local tree {label} (1,204 files, 3.1 MiB)"
                    )),
                ),
                source => log.fetch(&format!(
                    "git+ssh://forge/nix-config?rev={}",
                    short(source, 0)
                )),
            },
            Phase::Evaluating => {
                let platform = match host {
                    ALTAR | LAPTOP => Platform::Darwin,
                    _ => Platform::NixOs,
                };
                log.message(
                    3,
                    &format!(
                        "evaluating {}.{}.config.system.build.toplevel",
                        platform.configurations(),
                        HOSTS[host]
                    ),
                );
            }
            Phase::Building => {}
            Phase::Activating => {
                log.output("activating the configuration...");
                log.output("setting up /etc...");
                log.output("restarting the following units: coeusd.service, tailscaled.service");
            }
            Phase::Verifying => {
                log.event(
                    at,
                    JobEvent::Note("waiting for coeusd.service, sshd.service".into()),
                );
                log.event(at, JobEvent::Note("health checks passed".into()));
            }
        }
    }

    /// nix says how much there is to build and download
    fn start_work(&mut self, job: &mut Job) {
        let Some(work) = job.work else {
            return;
        };
        job.log.message(
            3,
            &format!("these {} derivations will be built:", work.builds.1),
        );
        job.log.message(
            3,
            &format!(
                "these {} paths will be fetched ({:.1} MiB download, {:.1} MiB unpacked):",
                work.downloads.1,
                work.bytes / MIB,
                work.bytes * 3.1 / MIB
            ),
        );
        job.progress = Progress {
            builds: Tally {
                done: work.builds.0,
                expected: work.builds.1,
                running: 0,
                failed: 0,
            },
            downloads: Tally {
                done: work.downloads.0,
                expected: work.downloads.1,
                running: 0,
                failed: 0,
            },
            downloaded_bytes: (work.bytes * f64::from(work.downloads.0)
                / f64::from(work.downloads.1)) as u64,
            expected_bytes: work.bytes as u64,
            active: Vec::new(),
        };
    }

    fn add_derivation(&mut self, job: &mut Job) {
        let name = loop {
            let name = *self.rng.pick(&PACKAGES);
            if job.active.iter().all(|(active, _)| active != name) {
                break name;
            }
        };
        let drv = format!("/nix/store/{}-{name}.drv", self.rng.hash());
        let id = job.log.build(&drv);
        job.active.push((name.to_owned(), id));
        job.progress.active = job.active.iter().map(|(name, _)| name.clone()).collect();
    }

    fn build(&mut self, host: usize, job: &mut Job, seconds: f64) {
        let Some(work) = job.work else {
            return;
        };
        let downloads = job.progress.downloads;
        if downloads.done < downloads.expected {
            job.downloaded += seconds * work.download_rate * (0.4 + self.rng.next() * 1.2);
            let more = job.downloaded.floor();
            if more > 0.0 {
                job.downloaded -= more;
                let before = downloads.done;
                let done = (before + more as u32).min(downloads.expected);
                for at in before..done {
                    if at % 9 == 0 {
                        let package = *self.rng.pick(&PACKAGES);
                        let path = format!("/nix/store/{}-{package}", self.rng.hash());
                        job.log.copy(&path);
                    }
                }
                job.progress.downloads.done = done;
            }
            let downloads = job.progress.downloads;
            job.progress.downloads.running = match downloads.done < downloads.expected {
                true => self.rng.between(2, 8),
                false => 0,
            };
            job.progress.downloaded_bytes = match downloads.done >= downloads.expected {
                true => job.progress.expected_bytes,
                false => {
                    let share = f64::from(downloads.done) / f64::from(downloads.expected);
                    let bytes = work.bytes * share * (0.985 + self.rng.next() * 0.015);
                    (bytes as u64).min(job.progress.expected_bytes)
                }
            };
        }

        let builds = job.progress.builds;
        if builds.done < builds.expected {
            job.built += seconds * work.build_rate * (0.3 + self.rng.next() * 1.4);
            let more = job.built.floor();
            if more > 0.0 {
                job.built -= more;
                job.progress.builds.done = (builds.done + more as u32).min(builds.expected);
            }
            job.rotate -= seconds;
            if job.rotate <= 0.0 && !job.active.is_empty() {
                let (_, id) = job.active.remove(0);
                job.log.stop(id);
                job.rotate = 0.7 + self.rng.next() * 1.3;
            }
            let builds = job.progress.builds;
            let wanted = (builds.expected - builds.done).min(4) as usize;
            while job.active.len() < wanted {
                self.add_derivation(job);
            }
            for (_, id) in job.active.split_off(wanted.min(job.active.len())) {
                job.log.stop(id);
            }
            job.progress.active = job.active.iter().map(|(name, _)| name.clone()).collect();
            job.progress.builds.running = job.active.len() as u32;
            if !job.active.is_empty() && self.rng.next() < seconds * 7.0 {
                let (name, id) = self.rng.pick(&job.active).clone();
                let line = *self.rng.pick(printed(&name));
                job.log.builder(id, line);
            }
        }

        let progress = &job.progress;
        if progress.builds.done >= progress.builds.expected
            && progress.downloads.done >= progress.downloads.expected
        {
            for (_, id) in std::mem::take(&mut job.active) {
                job.log.stop(id);
            }
            job.progress.active.clear();
            job.progress.builds.running = 0;
            job.progress.downloads.running = 0;
            self.enter(host, job, ACTIVATING);
        }
    }

    fn finish(&mut self, host: usize, mut job: Job, outcome: Outcome) {
        job.summary.state = JobState::Finished(outcome.clone());
        job.summary.finished_at = Some(Timestamp(self.now));
        job.log.event(self.now, JobEvent::Finished(outcome.clone()));

        let lands = outcome == Outcome::Succeeded
            && matches!(job.summary.action, Action::Switch | Action::Boot);
        if lands {
            let generation = self.machines[host].generation + 1;
            let current = self.store(host, &job.summary.source, generation);
            let machine = &mut self.machines[host];
            machine.generation = generation;
            machine.current = current;
            machine.deployed = job.summary.source.clone();
            if job.summary.action == Action::Boot {
                machine.pending_boot = true;
            }
            if machine.info.platform == Platform::Darwin {
                machine.info.version = "0.4.1".to_owned();
            }
            machine.paused = matches!(job.summary.source, Source::Snapshot { .. });
        }

        let rollout = job.rollout;
        let follow = job.summary.origin == Origin::Follow;
        let machine = &mut self.machines[host];
        machine.recent.insert(0, job);
        machine.recent.truncate(MachineView::RECENT);
        machine.job = machine.next.take().map(|mut next| {
            next.wait = 1.6;
            next
        });

        if rollout {
            self.rollout.verdicts[host] = match (outcome == Outcome::Succeeded, follow) {
                (true, true) => Verdict::CaughtUp,
                (true, false) => Verdict::Done,
                (false, _) => Verdict::Failed,
            };
            match follow {
                true => self.later(14.0, Timer::Restart),
                false => self.check_wave(),
            }
        }
    }

    fn check_wave(&mut self) {
        let wave = self.rollout.wave;
        let Some((hosts, ended)) = self.rollout.waves.get(wave) else {
            return;
        };
        let busy = hosts.iter().any(|host| {
            matches!(
                self.rollout.verdicts[*host],
                Verdict::Running | Verdict::Waiting | Verdict::CatchingUp
            )
        });
        if *ended || busy {
            return;
        }
        self.rollout.waves[wave].1 = true;
        if wave + 1 < self.rollout.waves.len() {
            self.rollout.wave += 1;
            self.later(1.8, Timer::StartWave);
        } else {
            self.later(6.0, Timer::Wake);
        }
    }

    fn fire(&mut self, timer: Timer) {
        match timer {
            Timer::StartWave => {
                let hosts = self.rollout.waves[self.rollout.wave].0.clone();
                for host in hosts {
                    if !self.machines[host].reachable {
                        self.rollout.verdicts[host] = Verdict::Unreachable;
                        continue;
                    }
                    self.rollout.verdicts[host] = Verdict::Running;
                    let (id, plan, work) = last_wave(host);
                    let mut job = self.ordered(
                        host,
                        id,
                        self.rollout.source.clone(),
                        self.rollout.origin.clone(),
                        plan,
                        work,
                    );
                    job.rollout = true;
                    job.wait = 0.6;
                    job.diff = Some(ClosureDiff {
                        changes: self.rollout.diffs[host].clone(),
                    });
                    self.machines[host].job = Some(job);
                }
                self.check_wave();
            }
            Timer::Wake => {
                let machine = &mut self.machines[LAPTOP];
                machine.reachable = true;
                machine.last_seen = self.now;
                self.rollout.verdicts[LAPTOP] = Verdict::CatchingUp;
                let (id, plan, work) = last_wave(LAPTOP);
                let mut job = self.ordered(
                    LAPTOP,
                    id,
                    source(&format!("deployed@{RELEASE}")),
                    Origin::Follow,
                    plan,
                    work,
                );
                job.rollout = true;
                job.wait = 1.4;
                job.diff = Some(ClosureDiff {
                    changes: self.rollout.diffs[LAPTOP].clone(),
                });
                self.machines[LAPTOP].job = Some(job);
            }
            Timer::Restart => self.ended = Some(self.now),
        }
    }

    pub fn hosts(&self) -> impl Iterator<Item = &Hostname> {
        self.machines.iter().map(|machine| &machine.info.hostname)
    }

    /// whether `host` answers, and when it last did
    pub fn reachable(&self, host: usize) -> (bool, Timestamp) {
        let machine = &self.machines[host];
        (machine.reachable, Timestamp(machine.last_seen))
    }

    /// what `host`'s coeusd reports
    pub fn view(&self, host: usize) -> MachineView {
        let machine = &self.machines[host];
        let (running, next) = match &machine.job {
            Some(job) if job.summary.state == JobState::Queued => (None, Some(job)),
            job => (job.as_ref(), machine.next.as_ref()),
        };
        MachineView {
            host: machine.info.clone(),
            system: SystemInfo {
                current: Some(machine.current.clone()),
                generation: Some(machine.generation),
                pending_boot: machine.pending_boot,
                reboot_required: machine.reboot_required,
                deployed: Some(machine.deployed.clone()),
                follow: Some(Follow {
                    git_ref: "deployed".to_owned(),
                    latest: Some(Pinned {
                        git_ref: "deployed".to_owned(),
                        rev: commit(RELEASE),
                        committed_at: Timestamp(self.base - (9.2 * MINUTE) as i64),
                    }),
                    paused: machine.paused,
                }),
                fda: match machine.info.platform {
                    Platform::Darwin => FdaStatus::Granted,
                    Platform::NixOs => FdaStatus::NotApplicable,
                },
                checked_at: Timestamp(self.base),
            },
            daemon: DaemonState::Serving,
            running: running.map(Job::view),
            next: next.map(|job| job.summary.clone()),
            recent: machine
                .recent
                .iter()
                .map(|job| job.summary.clone())
                .collect(),
        }
    }

    /// job `id` of `host`, wherever it sits, and its log
    pub fn job(&self, host: usize, id: u64) -> Option<(&JobSummary, &Log)> {
        let machine = &self.machines[host];
        machine
            .job
            .iter()
            .chain(&machine.next)
            .chain(&machine.recent)
            .find(|job| job.summary.id.0 == id)
            .map(|job| (&job.summary, &job.log))
    }
}

/// the log a finished job left, written by a generator of its own
fn written(summary: &JobSummary, host: &str, diff: Option<&ClosureDiff>) -> Log {
    let mut log = Log::default();
    let (Some(started), Some(finished), JobState::Finished(outcome)) =
        (summary.started_at, summary.finished_at, &summary.state)
    else {
        return log;
    };
    let seed = (summary.id.0 as u32)
        .wrapping_mul(7919)
        .wrapping_add(host.len() as u32 * 31);
    let mut rng = Rng(seed);

    let last = match outcome {
        Outcome::Failed(failure) | Outcome::RolledBack(failure) => PHASES
            .iter()
            .position(|phase| *phase == failure.phase)
            .unwrap_or(0),
        Outcome::Cancelled | Outcome::Superseded { .. } => 0,
        Outcome::UpToDate => 2,
        Outcome::Succeeded | Outcome::Interrupted => VERIFYING,
    };
    log.event(0, started_event());
    for (at, phase) in PHASES.iter().enumerate().take(last + 1) {
        log.event(0, JobEvent::Phase(*phase));
        if at != BUILDING {
            continue;
        }
        let lines = 14 + (rng.next() * 18.0) as usize;
        scroll(&mut rng, &mut log, lines, 0.4, 0.7);
        if let Some(diff) = diff {
            log.event(0, JobEvent::Diff(diff.clone()));
        }
    }

    if let Some(failure) = outcome.failure() {
        if failure.message.contains("zfs") {
            let id = log
                .build("/nix/store/1q9xvk2kq2gwzfwxjy6hn1nn2zka6v2c-zfs-kernel-2.2.7-6.12.9.drv");
            for line in [
                "CC [M]  module/zfs/arc.o",
                "module/os/linux/zfs/zpl_file.c:812:9: error: implicit declaration of function 'folio_mapping'",
                "make[3]: *** [scripts/Makefile.build:229: zpl_file.o] Error 1",
            ] {
                log.builder(id, line);
            }
            log.stop(id);
        }
        log.message(0, &failure.message);
    }
    log.event(0, JobEvent::Finished(outcome.clone()));
    log.spread(started.0, finished.0);
    log
}

/// `lines` lines of a build under way: paths copied, derivations started,
/// and what the builder started last printed
fn scroll(rng: &mut Rng, log: &mut Log, lines: usize, copies: f64, builds: f64) {
    let mut open: Option<(&str, u64)> = None;
    for _ in 0..lines {
        let package = *rng.pick(&PACKAGES);
        let roll = rng.next();
        let hash = rng.hash();
        let drv = format!("/nix/store/{hash}-{package}.drv");
        if roll < copies {
            log.copy(&format!("/nix/store/{hash}-{package}"));
        } else if roll < builds {
            if let Some((_, id)) = open.take() {
                log.stop(id);
            }
            open = Some((package, log.build(&drv)));
        } else {
            let (name, id) = *open.get_or_insert_with(|| (package, log.build(&drv)));
            log.builder(id, rng.pick(printed(name)));
        }
    }
    if let Some((_, id)) = open {
        log.stop(id);
    }
}

fn started_event() -> JobEvent {
    JobEvent::Started {
        attempt: 1,
        pid: 4242,
        version: "0.4.1".to_owned(),
    }
}
