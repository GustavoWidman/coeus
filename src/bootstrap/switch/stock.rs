//! the first switch over ssh: ship, build the fleet's coeus, adopt, and on a
//! NixOS that refuses to switch live, adopt a boot and reboot

use std::{
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

use eyre::{Result, WrapErr, bail, eyre};
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, BufReader},
    process::Command,
};

use super::{
    AdoptLine, Feed, Release,
    backoff::Backoff,
    first::{self, Stocked},
    ship::{Archived, Lock},
    show,
};
use crate::{
    adopt::{DAEMON_HERE, NEEDS_FULL_DISK_ACCESS},
    bootstrap::{
        Blocker, Bootstrap, Step,
        remote::{AdoptCall, Call, Platform, Privilege, Running, Ssh, Stock},
    },
    cli::Console,
    client::Snapshot,
    common::model::{
        self, Action, Attempt, Hostname, JobSummary, NarHash, OrderId, StorePath, Timestamp,
    },
    nix::LogFold,
    ui::checklist::{Live, Mark, Row},
};

/// how a stream of `coeusd adopt --follow` ended, or the first switch with
/// it
pub enum Ending {
    Finished(JobSummary),
    /// the boot job succeeded and the machine was told to reboot into its
    /// system, from the boot `before` names
    Rebooted {
        job: JobSummary,
        before: Running,
    },
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

/// ships, builds the fleet's coeus, then reads where the switch stands and
/// adopts what comes next until the machine runs the release, a try fails,
/// or it reboots into the release. the coeus and the source come back for
/// following a job again after ssh drops.
pub async fn switch(
    stock: &Stock,
    plan: &Plan<'_>,
    feed: &mut Feed,
    console: &Console,
) -> Result<(FleetCoeus, StorePath, Ending)> {
    let source = ship(stock, plan, console).await?;
    let coeus = fleet_coeus(stock, &source, plan.host, console).await?;
    let platform = model::Platform::from(stock.probed.facts.platform);
    let mut over = OverSsh {
        stock,
        coeus: &coeus,
        source: &source,
        plan,
        feed,
        console,
    };
    let ending = first::converge(&mut over, plan.host, platform, plan.retry, console).await?;
    Ok((coeus, source, ending))
}

/// the stock machine over ssh, as the first switch goes through it
struct OverSsh<'a, 'p> {
    stock: &'a Stock,
    coeus: &'a FleetCoeus,
    source: &'a StorePath,
    plan: &'a Plan<'p>,
    feed: &'a mut Feed,
    console: &'a Console,
}

impl Stocked for OverSsh<'_, '_> {
    async fn status(&mut self) -> Result<Attempt> {
        status(self.stock, self.coeus, self.source, self.plan).await
    }

    async fn running(&mut self) -> Result<Running> {
        Running::parse(&self.stock.run(&Call::Running).await?)
    }

    async fn adopt(&mut self, order: OrderId, action: Action) -> Result<Ending> {
        *self.feed = Feed::default();
        adopt(
            self.stock,
            self.coeus,
            self.source,
            self.plan,
            order,
            action,
            0,
            self.feed,
            self.console,
        )
        .await
    }

    async fn reboot(&mut self) -> Result<()> {
        reboot(self.stock, self.plan.host, self.console).await
    }
}

/// `systemctl reboot` as root. ssh dropping as the machine goes down is
/// the reboot under way.
pub async fn reboot(stock: &Stock, host: &Hostname, console: &Console) -> Result<()> {
    console.mark(
        Step::Switch,
        Mark::Active,
        format!("rebooting {host} into its fleet system"),
    );
    let output = stock.output(&Call::Reboot).await?;
    if output.status.success() || Ssh::lost(output.status) {
        return Ok(());
    }
    bail!(
        "could not reboot {host}: {}",
        String::from_utf8_lossy(&output.stderr)
            .lines()
            .last()
            .unwrap_or("systemctl said nothing")
    );
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

    let program = remote_program(stock.probed.facts.platform, &stock.privilege);
    let (store, ssh_options) = stock.ssh.nix_store(&program);
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
        let why = fold.last_error().unwrap_or("nix copy failed");
        // sudo that asks a password cannot run nix's daemon, so a login
        // that is not root ships as itself, which only a trusted user may
        if matches!(stock.privilege, Privilege::SudoPassword(_)) && why.contains("signature") {
            return Err(Blocker::RootRefused {
                user: stock.probed.facts.login_user.clone(),
            }
            .into());
        }
        bail!("could not ship the clone to {host}: {why}");
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
/// switch row until the job finishes or the stream ends; `action` is what a
/// job admitted now does
#[allow(clippy::too_many_arguments)]
pub async fn adopt(
    stock: &Stock,
    coeus: &FleetCoeus,
    source: &StorePath,
    plan: &Plan<'_>,
    order: OrderId,
    action: Action,
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
        boot: action == Action::Boot,
        install_bootloader: plan.release.install_bootloader,
        follow: Some(from),
    });
    let mut asked = None;
    let mut waits = Backoff::new(Duration::from_secs(5), Duration::from_secs(30));
    let (status, said) = loop {
        let (status, said) = run_adopt(stock, &call, feed, console).await?;
        match Waiting::after(status, asked.is_some(), feed) {
            Waiting::Over => break (status, said),
            Waiting::ForGrant => {
                let first = asked.is_none();
                let since = *asked.get_or_insert_with(Timestamp::now);
                ask_for_full_disk_access(stock, plan.host, &said, since, first, console).await;
            }
            Waiting::ForSsh => {}
        }
        tokio::select! {
            () = tokio::time::sleep(waits.next()) => {}
            () = console.cancelled.cancelled() => {
                bail!("detached while waiting for Full Disk Access on {}", plan.host);
            }
        }
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
        _ if action == Action::Boot && said.contains("'--boot'") => Err(eyre!(
            "NixOS on {} refuses to switch live, and the coeus its configuration pins cannot \
             `coeusd adopt --boot`: update coeus on the branch",
            plan.host
        )),
        _ => Err(eyre!(
            "coeusd adopt on {} failed: {}",
            plan.host,
            said.lines().last().unwrap_or("it said nothing")
        )),
    }
}

/// what one adopt says about waiting for a person to grant Full Disk Access
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Waiting {
    /// adopt admitted or followed the job, or failed otherwise
    Over,
    /// the launcher still lacks the grant
    ForGrant,
    /// ssh dropped while waiting, which says nothing about the grant
    ForSsh,
}

impl Waiting {
    pub(super) fn after(status: std::process::ExitStatus, waiting: bool, feed: &Feed) -> Self {
        match status.code() {
            Some(NEEDS_FULL_DISK_ACCESS) => Self::ForGrant,
            _ if waiting && feed.view.is_none() && Ssh::lost(status) => Self::ForSsh,
            _ => Self::Over,
        }
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

/// what nix's ssh store runs on the machine: nix-daemon by its path, as
/// root, so the unsigned inputs are taken. a login that is not root and
/// may sudo without a password runs it through sudo.
pub(super) fn remote_program(platform: Platform, privilege: &Privilege) -> String {
    let daemon = match platform {
        Platform::Nixos => "/run/current-system/sw/bin/nix-daemon",
        Platform::Darwin => "/nix/var/nix/profiles/default/bin/nix-daemon",
    };
    match privilege {
        Privilege::Sudo => format!("sudo -n {daemon}"),
        Privilege::Root | Privilege::SudoPassword(_) => daemon.to_owned(),
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

    let stdout = child.stdout.take().expect("stdout is piped");
    drain(stdout, feed, |feed| show(console, feed, None)).await;
    let status = child.wait().await?;
    Ok((status, said.await.unwrap_or_default()))
}

/// every line adopt prints, folded into `feed`, until it stops: adopt goes
/// on to the job's finishing event even when its first line says the job
/// finished, and output left unread fills the pipe and ssh's window, so
/// adopt would never exit
pub(super) async fn drain(
    stdout: impl AsyncRead + Unpin,
    feed: &mut Feed,
    mut shown: impl FnMut(&Feed),
) {
    let mut lines = BufReader::new(stdout).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        feed.line(AdoptLine::parse(&line));
        shown(feed);
    }
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
