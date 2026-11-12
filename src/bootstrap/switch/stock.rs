//! the first switch over ssh: ship, build the fleet's coeus, adopt

use std::{
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

use eyre::{Result, WrapErr, bail, eyre};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, BufReader},
    process::Command,
};

use super::{
    AdoptLine, Feed, Release,
    ship::{Archived, Lock},
    show,
};
use crate::{
    adopt::{DAEMON_HERE, NEEDS_FULL_DISK_ACCESS},
    bootstrap::{
        Blocker, Bootstrap, Step,
        remote::{AdoptCall, Call, Platform, Ssh, Stock},
    },
    cli::Console,
    client::Snapshot,
    common::model::{Attempt, Hostname, JobSummary, NarHash, OrderId, StorePath, Timestamp},
    nix::LogFold,
    ui::checklist::{Live, Mark, Row},
};

/// how a stream of `coeusd adopt --follow` ended
pub enum Ending {
    Finished(JobSummary),
    /// the stream ended without the job finishing, at this log offset
    Lost {
        job: JobSummary,
        offset: u64,
    },
    /// adopt exited 3: a coeusd serves on the machine, order through it
    DaemonHere,
}

/// the coeus the machine's configuration pins, built on the machine
pub struct FleetCoeus {
    pub path: StorePath,
    pub version: String,
}

/// what the first switch is decided on, besides the machine
pub struct Plan<'a> {
    pub release: &'a Release,
    pub host: &'a Hostname,
    /// the private clone the release is the committed head of
    pub clone: PathBuf,
    pub retry: bool,
}

impl<'a> Plan<'a> {
    pub fn of(release: &'a Release, bootstrap: &'a Bootstrap) -> Self {
        Self {
            release,
            host: &bootstrap.host,
            clone: bootstrap.state.clone_dir(),
            retry: bootstrap.options.retry,
        }
    }
}

/// ships, builds the fleet's coeus, reads where the switch stands and
/// adopts the order that comes next, following it from the start. the coeus
/// and the source come back for following it again after ssh drops.
pub async fn switch(
    stock: &Stock,
    plan: &Plan<'_>,
    feed: &mut Feed,
    console: &Console,
) -> Result<(FleetCoeus, StorePath, Ending)> {
    let source = ship(stock, plan, console).await?;
    let coeus = fleet_coeus(stock, &source, plan.host, console).await?;
    let order = match status(stock, &coeus, &source, plan).await? {
        Attempt::Fresh(order) => order,
        Attempt::Running(job) | Attempt::Succeeded(job) => job.order,
        Attempt::Failed { last, next } => match (plan.retry, next) {
            (true, Some(next)) => next,
            (true, None) => {
                return Err(eyre!(
                    "every try of this head failed on {}; commit a fix in the clone and run again",
                    plan.host
                ));
            }
            (false, _) => {
                return Err(Blocker::SwitchFailed {
                    job: Box::new(last),
                }
                .into());
            }
        },
    };
    let ending = adopt(stock, &coeus, &source, plan, order, 0, feed, console).await?;
    Ok((coeus, source, ending))
}

/// the clone and the inputs only this admin can reach, copied into the
/// machine's store through the pinned master: the machine then evaluates
/// what this admin does without any of the fleet's credentials, and fetches
/// the public inputs its evaluation touches itself. a rerun copies only what
/// is missing. returns the source's store path there.
pub async fn ship(stock: &Stock, plan: &Plan<'_>, console: &Console) -> Result<StorePath> {
    let host = plan.host;
    let stage = Snapshot::stage(&plan.clone).await?;
    let staged = nar_hash(stage.path()).await?;
    if staged != plan.release.nar_hash {
        bail!(
            "the clone holds {staged}, not the {} the switch is for: run again",
            plan.release.nar_hash
        );
    }

    console.mark(Step::Switch, Mark::Active, "reading the clone's inputs");
    let flake = format!("path:{}", stage.path().display());
    let archived = Archived::parse(
        &nix_output(&[
            "flake",
            "archive",
            "--json",
            "--no-update-lock-file",
            &flake,
        ])
        .await
        .wrap_err("could not archive the clone on this machine")?,
    )?;
    let paths = match tokio::fs::read_to_string(stage.path().join("flake.lock")).await {
        Ok(lock) => Lock::parse(&lock)?.private_paths(&archived),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => vec![archived.path.clone()],
        Err(error) => return Err(error).wrap_err("could not read the clone's flake.lock"),
    };

    let (store, ssh_options) = stock.ssh.nix_store(nix_daemon(stock.probed.facts.platform));
    let what = match paths.len() - 1 {
        0 => "the clone".to_owned(),
        1 => "the clone and the input only this machine reaches".to_owned(),
        inputs => format!("the clone and the {inputs} inputs only this machine reaches"),
    };
    // inputs this admin substituted carry signatures of caches a stock machine
    // does not trust; the lock pins each by its nar hash, which the machine
    // checks when it evaluates, so root on the machine takes them unsigned
    let mut child = Command::new("nix")
        .args(["--extra-experimental-features", crate::nix::FLAKE_FEATURES])
        .args(["copy", "--no-check-sigs", "--log-format", "internal-json"])
        .args(["--to", &store])
        .args(paths.iter().map(ToString::to_string))
        .env("NIX_SSHOPTS", ssh_options)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .wrap_err("could not run nix on this machine")?;

    console.mark(
        Step::Switch,
        Mark::Active,
        format!("shipping {what} to {host}"),
    );
    let mut fold = LogFold::new();
    let mut lines = BufReader::new(child.stderr.take().expect("stderr is piped")).lines();
    while let Some(line) = lines.next_line().await? {
        fold.feed(&line);
        if fold.snapshot().is_some() {
            let copied = fold.progress().downloads;
            console.mark(
                Step::Switch,
                Mark::Active,
                format!(
                    "shipping {what} to {host}: {} of {} paths",
                    copied.done, copied.expected
                ),
            );
        }
    }
    if !child.wait().await?.success() {
        bail!(
            "could not ship the clone to {host}: {}",
            fold.last_error().unwrap_or("nix copy failed")
        );
    }
    Ok(archived.path)
}

/// what a local nix prints, or its last error line
async fn nix_output(args: &[&str]) -> Result<String> {
    let output = Command::new("nix")
        .args(["--extra-experimental-features", crate::nix::FLAKE_FEATURES])
        .args(args)
        .stdin(Stdio::null())
        .kill_on_drop(true)
        .output()
        .await?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!(
            "{}",
            stderr
                .lines()
                .rev()
                .find(|line| !line.trim().is_empty())
                .unwrap_or("nix failed")
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// `<configurations>.<host>.config.services.coeus.package` built as root,
/// the first evaluation of the machine's configuration, so its errors are
/// the person's to read
pub async fn fleet_coeus(
    stock: &Stock,
    source: &StorePath,
    host: &Hostname,
    console: &Console,
) -> Result<FleetCoeus> {
    let platform = crate::common::model::Platform::from(stock.probed.facts.platform);
    let call = Call::BuildCoeus {
        source: source.clone(),
        attribute: format!(
            "{}.{host}.config.services.coeus.package",
            platform.configurations()
        ),
    };
    let detail = format!("building the coeus {host}'s configuration pins");
    console.mark(Step::Switch, Mark::Active, &detail);

    let mut fold = LogFold::new();
    let mut printed = String::new();
    let status = stock
        .follow(&call, |line| {
            if !line.starts_with(crate::job::JobLog::NIX) {
                printed.push_str(line);
                return;
            }
            fold.feed(line);
            if fold.snapshot().is_some() {
                console.update(Row {
                    step: Step::Switch,
                    mark: Mark::Active,
                    detail: detail.clone(),
                    live: build_lines(&fold),
                });
            }
        })
        .await?;
    if !status.success() {
        if Ssh::lost(status) {
            bail!("the connection to {host} dropped while building its coeus");
        }
        let mut why = fold.last_error().unwrap_or("nix build failed").to_owned();
        let tail: Vec<&str> = fold.tail().collect();
        for line in &tail[tail.len().saturating_sub(3)..] {
            why.push_str("\n  ");
            why.push_str(line);
        }
        bail!("{host}'s configuration does not build its coeus: {why}");
    }

    let path = built_path(&printed)?;
    // the store path names the version, even of a coeus older than its
    // `--version`
    let version = path
        .as_str()
        .rsplit_once("-coeus-")
        .map_or("unknown", |(_, version)| version)
        .to_owned();
    Ok(FleetCoeus { path, version })
}

/// `coeusd adopt --status`, where this snapshot's switch stands on the
/// machine, read in the state directory the configuration in `source`
/// names. a coeus without `adopt` blocks, naming its version.
pub async fn status(
    stock: &Stock,
    coeus: &FleetCoeus,
    source: &StorePath,
    plan: &Plan<'_>,
) -> Result<Attempt> {
    let call = Call::AdoptStatus {
        coeus: coeus.path.clone(),
        nar_hash: plan.release.nar_hash.clone(),
        source: source.as_str().into(),
        hostname: plan.host.clone(),
    };
    let output = stock.output(&call).await?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !output.status.success() {
        if stderr.contains("unrecognized subcommand") {
            return Err(Blocker::AdoptMissing {
                version: coeus.version.clone(),
            }
            .into());
        }
        bail!(
            "coeusd adopt --status failed: {}",
            stderr.lines().last().unwrap_or_default()
        );
    }
    serde_json::from_slice(&output.stdout)
        .map_err(|error| eyre!("coeusd adopt --status printed something else: {error}"))
}

/// runs `coeusd adopt … --follow <from>` as root and feeds its lines to the
/// switch row until the job finishes or the stream ends
#[allow(clippy::too_many_arguments)]
pub async fn adopt(
    stock: &Stock,
    coeus: &FleetCoeus,
    source: &StorePath,
    plan: &Plan<'_>,
    order: OrderId,
    from: u64,
    feed: &mut Feed,
    console: &Console,
) -> Result<Ending> {
    let call = Call::Adopt(AdoptCall {
        coeus: coeus.path.clone(),
        source: source.as_str().into(),
        nar_hash: plan.release.nar_hash.clone(),
        label: plan.release.label.clone(),
        hostname: plan.host.clone(),
        order,
        install_bootloader: plan.release.install_bootloader,
        follow: Some(from),
    });
    let mut asked = None;
    let (status, said) = loop {
        let (status, said) = run_adopt(stock, &call, feed, console).await?;
        if status.code() != Some(NEEDS_FULL_DISK_ACCESS) {
            break (status, said);
        }
        let first = asked.is_none();
        let since = *asked.get_or_insert_with(Timestamp::now);
        ask_for_full_disk_access(stock, plan.host, &said, since, first, console).await;
        tokio::time::sleep(Duration::from_secs(5)).await;
    };

    if let Some(job) = feed.finished() {
        return Ok(Ending::Finished(job.clone()));
    }
    if status.code() == Some(DAEMON_HERE) {
        return Ok(Ending::DaemonHere);
    }
    match &feed.view {
        Some(view) if Ssh::lost(status) || status.success() => Ok(Ending::Lost {
            job: view.summary.clone(),
            offset: feed.offset,
        }),
        _ => Err(eyre!(
            "coeusd adopt on {} failed: {}",
            plan.host,
            said.lines().last().unwrap_or("it said nothing")
        )),
    }
}

/// the build's progress, and what it builds, under the row
fn build_lines(fold: &LogFold) -> Vec<Live> {
    let progress = fold.progress();
    let mut lines = Vec::new();
    if !progress.builds.is_empty() || !progress.downloads.is_empty() {
        lines.push(format!(
            "{} of {} built, {} of {} fetched",
            progress.builds.done,
            progress.builds.expected,
            progress.downloads.done,
            progress.downloads.expected
        ));
    }
    lines.extend(progress.active.iter().cloned());
    match lines.is_empty() {
        true => Vec::new(),
        false => vec![Live::Tail(lines)],
    }
}

/// where nix-daemon is on the machine, for nix's ssh store
fn nix_daemon(platform: Platform) -> &'static str {
    match platform {
        Platform::Nixos => "/run/current-system/sw/bin/nix-daemon",
        Platform::Darwin => "/nix/var/nix/profiles/default/bin/nix-daemon",
    }
}

async fn nar_hash(dir: &Path) -> Result<NarHash> {
    let output = Command::new("nix")
        .args(["--extra-experimental-features", "nix-command"])
        .args(["hash", "path", "--type", "sha256", "--sri"])
        .arg(dir)
        .output()
        .await?;
    if !output.status.success() {
        bail!(
            "nix hash path failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    NarHash::try_from(String::from_utf8(output.stdout)?.trim().to_owned())
}

/// `[{"drvPath": …, "outputs": {"out": "/nix/store/…"}}]`
pub(super) fn built_path(printed: &str) -> Result<StorePath> {
    let built: serde_json::Value = serde_json::from_str(printed.trim())
        .map_err(|error| eyre!("nix build printed something else: {error}"))?;
    let out = built[0]["outputs"]["out"]
        .as_str()
        .ok_or_else(|| eyre!("nix build named no output"))?;
    out.parse()
}

/// one `coeusd adopt --follow`, its lines folded into `feed` as they come
async fn run_adopt(
    stock: &Stock,
    call: &Call,
    feed: &mut Feed,
    console: &Console,
) -> Result<(std::process::ExitStatus, String)> {
    let mut child = stock.spawn(call)?;
    let mut stderr = child.stderr.take().expect("stderr is piped");
    let said = tokio::spawn(async move {
        let mut text = String::new();
        let _ = stderr.read_to_string(&mut text).await;
        text
    });

    let mut lines = BufReader::new(child.stdout.take().expect("stdout is piped")).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        feed.line(AdoptLine::parse(&line));
        show(console, feed, None);
        if feed.finished().is_some() {
            break;
        }
    }
    let status = child.wait().await?;
    Ok((status, said.await.unwrap_or_default()))
}

/// macOS lets a launchd job write where nix-darwin's first activation
/// writes only with Full Disk Access, and only a person at the Mac grants
/// it: the pane and the launcher are opened on its screen the first time,
/// and the row says what to do until adopt finds the grant
async fn ask_for_full_disk_access(
    stock: &Stock,
    host: &Hostname,
    said: &str,
    since: Timestamp,
    first: bool,
    console: &Console,
) {
    let launcher = said
        .lines()
        .find_map(|line| line.split_once(" needs Full Disk Access"))
        .map_or(crate::client::Checkup::LAUNCHER, |(path, _)| path.trim());
    if first {
        let open = Call::OpenFullDiskAccess {
            launcher: launcher.to_owned(),
        };
        let _ = stock.run(&open).await;
    }
    console.update(Row {
        step: Step::Switch,
        mark: Mark::Active,
        detail: format!("waiting for Full Disk Access on {host}"),
        live: vec![
            Live::Waiting {
                since,
                on: format!("someone at {host}"),
            },
            Live::Tail(vec![
                "nix-darwin's first activation writes where only Full Disk Access may".to_owned(),
                format!("on {host}: System Settings > Privacy & Security > Full Disk Access"),
                format!("add {launcher}, which Finder shows, and turn it on"),
                "this goes on by itself once it is granted".to_owned(),
            ]),
        ],
    });
}
