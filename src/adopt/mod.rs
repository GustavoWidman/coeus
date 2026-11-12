//! `coeusd adopt`: admits a job on a machine no coeusd runs on yet, the way
//! coeusd admits one, so the coeusd the switch installs finds it on disk,
//! follows its runner and keeps it in its history. it runs from the coeus
//! the machine's configuration pins, so the job directory is written by the
//! same code that will read it.
//!
//! it is idempotent: the order id comes from the snapshot, so running it
//! again finds the job, and does to an unfinished one what coeusd does at
//! startup (follow it, start a runner again, or close it by what the
//! machine runs).

mod admit;
mod follow;
mod settings;

#[cfg(test)]
mod test;

use std::{fs, os::unix::fs::DirBuilderExt, path::PathBuf};

use eyre::{Result, WrapErr, eyre};
use log::{info, warn};

use self::admit::{Admitted, Revival};
use crate::{
    cli::daemon::AdoptArgs,
    common::model::{
        Action, Attempt, Failure, FdaStatus, HostInfo, Hostname, JobId, NarHash, NixOptions, Order,
        OrderId, Origin, Outcome, Phase, Platform, Source, StorePath,
    },
    config::DaemonSettings,
    engine::{Host, Snapshots},
    job::{JobDir, JobSpec, JobStore},
    platform::{Launcher, Machine, SystemRoot},
};

/// what adopt exits with when a coeusd already serves here: the CLI orders
/// through it instead
pub const DAEMON_HERE: i32 = 3;

/// what adopt exits with on a Mac whose launcher lacks Full Disk Access: a
/// first activation writes where only it may, and only a person grants it
pub const NEEDS_FULL_DISK_ACCESS: i32 = 4;

/// who an adopted job's history names as having ordered it, from the host
/// the label names
const ORDERED_BY: &str = "bootstrap";

/// the process's exit code
pub async fn run(args: &AdoptArgs) -> Result<i32> {
    if args.status {
        let state_dir = args.state_dir.clone().unwrap_or_else(default_state_dir);
        let store = JobStore::open(&state_dir)?;
        let attempt = Attempt::of(&admit::summaries(&store).await?, &args.nar_hash);
        println!("{}", serde_json::to_string(&attempt)?);
        return Ok(0);
    }

    let request = Request::parse(args)?;
    let settings = settings::of(&request.source, &request.hostname).await?;
    let port = settings.listen.port();
    if settings::listening(port).await {
        eprintln!("coeusd already serves on port {port}: order through it");
        return Ok(DAEMON_HERE);
    }

    if request.lacks_full_disk_access(&settings).await? {
        eprintln!(
            "{} needs Full Disk Access before its first activation",
            settings.launcher.display()
        );
        return Ok(NEEDS_FULL_DISK_ACCESS);
    }

    let dir = request.admit(&settings).await?;
    let mut stdout = tokio::io::stdout();
    match request.follow {
        Some(from) => {
            follow::stream(&dir, from, &mut stdout).await?;
        }
        None => {
            let summary = dir.view().await?.summary;
            println!("@job {}", serde_json::to_string(&summary)?);
        }
    }
    Ok(0)
}

/// adopt's flags, all present
struct Request {
    source: StorePath,
    nar_hash: NarHash,
    label: String,
    hostname: Hostname,
    order: OrderId,
    install_bootloader: bool,
    follow: Option<u64>,
}

impl Request {
    fn parse(args: &AdoptArgs) -> Result<Self> {
        let missing = |flag: &str| eyre!("--{flag} is required");
        Ok(Self {
            source: args.source.clone().ok_or_else(|| missing("source"))?,
            nar_hash: args.nar_hash.clone(),
            label: args.label.clone().ok_or_else(|| missing("label"))?,
            hostname: args.hostname.clone().ok_or_else(|| missing("hostname"))?,
            order: args.order.ok_or_else(|| missing("order"))?,
            install_bootloader: args.install_bootloader,
            follow: args.follow,
        })
    }

    /// on a Mac about to admit the order's job, whether the launcher it runs
    /// through still lacks the grant. a job admitted already goes its way.
    async fn lacks_full_disk_access(&self, settings: &DaemonSettings) -> Result<bool> {
        if Platform::current() != Platform::Darwin {
            return Ok(false);
        }
        create_private(&settings.state_dir)?;
        if JobStore::open(&settings.state_dir)?
            .find(self.order)?
            .is_some()
        {
            return Ok(false);
        }
        let machine = Machine::new(settings)?;
        Ok(machine.probe_fda().await == FdaStatus::Missing)
    }

    /// places the snapshot, finds or admits the order's job and sees that
    /// a runner carries it out
    async fn admit(&self, settings: &DaemonSettings) -> Result<JobDir> {
        let state_dir = &settings.state_dir;
        create_private(state_dir)?;

        // one adopt at a time places the snapshot and decides what the
        // order's job is; one a killed run left behind may still be at it
        let lock = fs::File::create(state_dir.join("adopt.lock"))?;
        lock.lock()?;

        Snapshots::new(state_dir)
            .place(&self.source, &self.nar_hash)
            .await?;

        let host = HostInfo::detect(Some(self.hostname.clone()))?;
        let exe = std::env::current_exe()?
            .canonicalize()
            .wrap_err("failed to locate the coeusd binary")?;
        let launcher = Launcher::new(host.platform, &settings.launcher, state_dir, &exe)?;
        let store = JobStore::open(state_dir)?;

        let spec = |id| self.spec(id, &host, settings);
        let current = SystemRoot::default().current();
        let dir = match admit::admit(&store, &self.nar_hash, self.order, spec).await? {
            Admitted::Created(dir) => {
                info!("admitted job {} for order {}", dir.id(), self.order);
                dir
            }
            Admitted::Found(dir) => match admit::revive(&dir, current.as_ref()).await? {
                Revival::Spawn => {
                    info!("job {} has no runner, starting another", dir.id());
                    dir
                }
                Revival::Closed(outcome) => {
                    warn!("closed job {} left unfinished: {outcome}", dir.id());
                    return Ok(dir);
                }
                Revival::Finished | Revival::Running => return Ok(dir),
            },
        };

        if let Err(error) = launcher.run_job(&exe, &dir).await {
            let message = format!("cannot start a runner: {error:#}");
            dir.close(Outcome::Failed(Failure::new(Phase::Preparing, message)))?;
        }
        Ok(dir)
    }

    /// the job coeusd would have written for this order
    fn spec(&self, id: JobId, host: &HostInfo, settings: &DaemonSettings) -> JobSpec {
        let admin = self
            .label
            .split_once(':')
            .map_or(&*self.label, |(admin, _)| admin);
        let order = Order {
            id: self.order,
            source: Source::Snapshot {
                nar_hash: self.nar_hash.clone(),
                label: self.label.clone(),
            },
            action: Action::Switch,
            options: NixOptions::default(),
            origin: Origin::User {
                user: ORDERED_BY.to_owned(),
                host: admin.to_owned(),
            },
            now: false,
        };

        JobSpec {
            install_bootloader: self.install_bootloader,
            ..JobSpec::admit(id, order, host, settings)
        }
    }
}

/// the state directory `--status` reads when not told another, coeusd's
fn default_state_dir() -> PathBuf {
    DaemonSettings::default().state_dir
}

fn create_private(dir: &std::path::Path) -> Result<()> {
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
        .wrap_err_with(|| format!("failed to create {}", dir.display()))
}
