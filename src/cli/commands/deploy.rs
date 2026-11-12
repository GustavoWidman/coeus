use std::{num::NonZeroUsize, path::PathBuf, sync::Arc};

use clap::Args;
use eyre::{Result, bail};

use crate::{
    cli::{
        Context,
        context::local_hostname,
        deployment::{Deployment, Following},
        dispatch::Shipment,
        exit::Exit,
        rollout::Rollout,
    },
    client::{Fleet, Snapshot, detect_origin},
    common::model::{Action, CommitSha, NixOptions, Order, OrderId, Source},
    ui::{board::Verdict, format, say},
};

#[derive(Args, Debug)]
pub struct DeployArgs {
    /// Machines to deploy [default: this machine]
    #[arg(value_name = "TARGETS")]
    pub targets: Vec<String>,

    /// Deploy this commit of the fleet's flake repository
    #[arg(long, value_name = "SHA", conflicts_with = "git_ref")]
    pub rev: Option<CommitSha>,

    /// Deploy the commit this ref points to now, the same on every machine
    #[arg(long = "ref", value_name = "REF", default_value = "main")]
    pub git_ref: String,

    /// Deploy a working tree as it is, uncommitted changes included
    #[arg(
        long,
        value_name = "DIR",
        num_args = 0..=1,
        default_missing_value = ".",
        conflicts_with_all = ["rev", "git_ref"]
    )]
    pub local: Option<PathBuf>,

    #[command(flatten)]
    pub action: ActionArgs,

    /// Replace a running job that has not started activating
    #[arg(long)]
    pub now: bool,

    /// Skip nix's evaluation cache
    #[arg(long)]
    pub no_eval_cache: bool,

    /// Ignore stale substituters left by an older generation
    #[arg(long)]
    pub clean_substituters: bool,

    /// Print each machine's answer and exit; the jobs run on their own
    #[arg(short, long, conflicts_with_all = ["canary", "batch", "notify", "no_attach"])]
    pub detach: bool,

    /// Deploy these machines first and stop if they fail
    #[arg(long, value_name = "TARGETS", value_delimiter = ',')]
    pub canary: Vec<String>,

    /// Deploy the remaining machines this many at a time
    #[arg(long, value_name = "N")]
    pub batch: Option<NonZeroUsize>,

    /// Stop ordering more machines once more than this many failed
    #[arg(long, value_name = "N", default_value_t = 0)]
    pub max_failures: usize,

    /// Show a desktop notification when done
    #[arg(long)]
    pub notify: bool,

    /// Exit successfully even when machines could not be reached
    #[arg(long)]
    pub skip_unreachable: bool,

    /// Show the fleet view for a single machine instead of its build
    #[arg(long)]
    pub no_attach: bool,
}

/// what to do with the built system
#[derive(Args, Debug)]
#[group(multiple = false)]
pub struct ActionArgs {
    /// Make it the boot default without activating it (NixOS)
    #[arg(long)]
    pub boot: bool,

    /// Activate it without making it the boot default (NixOS)
    #[arg(long)]
    pub test: bool,

    /// Only build it and show what it changes
    #[arg(long)]
    pub build: bool,
}

impl ActionArgs {
    const fn action(&self) -> Action {
        match (self.boot, self.test, self.build) {
            (true, _, _) => Action::Boot,
            (_, true, _) => Action::Test,
            (_, _, true) => Action::Build,
            _ => Action::Switch,
        }
    }
}

impl DeployArgs {
    pub async fn run(self, context: &Context) -> Result<Exit> {
        let targets = match self.targets.is_empty() {
            true => context.targets(&[local_hostname()])?,
            false => context.targets(&self.targets)?,
        };
        let canaries = match self.canary.is_empty() {
            true => Vec::new(),
            false => context.targets(&self.canary)?.hosts().to_vec(),
        };
        let fleet = context.fleet(targets)?;

        let snapshot = match &self.local {
            Some(dir) => Some(Arc::new(pack(dir).await?)),
            None => None,
        };
        let source = match &snapshot {
            Some(snapshot) => snapshot.source(),
            None => match self.pin(&fleet).await? {
                Some(source) => source,
                None => return Ok(Exit::of([Verdict::Unreachable], self.skip_unreachable)),
            },
        };

        let rollout = Rollout::plan(
            fleet.hosts(),
            &canaries,
            self.batch,
            self.max_failures,
            self.skip_unreachable,
        )?;
        let waves = rollout.waves().len();

        let following = match () {
            _ if self.detach => Following::Detach,
            _ if fleet.hosts().len() == 1 && waves == 1 && context.live && !self.no_attach => {
                Following::Attach
            }
            _ => Following::Board { live: context.live },
        };

        let order = Order {
            id: OrderId::random(),
            source,
            action: self.action.action(),
            options: NixOptions {
                no_eval_cache: self.no_eval_cache,
                clean_substituters: self.clean_substituters,
            },
            origin: detect_origin(),
            now: self.now,
        };

        Deployment {
            fleet,
            shipment: Shipment { order, snapshot },
            rollout,
            following,
            notify: self.notify,
            skip_unreachable: self.skip_unreachable,
        }
        .run()
        .await
    }

    /// the commit to deploy, pinned once on the first machine that answers
    /// so the whole order builds the same thing; `None` when none answered
    async fn pin(&self, fleet: &Fleet) -> Result<Option<Source>> {
        if let Some(rev) = self.rev {
            return Ok(Some(Source::Git { rev, git_ref: None }));
        }

        let mut failures = Vec::new();
        for host in fleet.hosts() {
            let pinned = match fleet.connect(host).await {
                Ok(mut client) => client.resolve(&self.git_ref).await,
                Err(error) => Err(error),
            };
            match pinned {
                Ok(pinned) => return Ok(Some(pinned.into())),
                Err(error) => failures.push(format!("{host}: {error}")),
            }
        }

        if failures.is_empty() {
            bail!("no machine to resolve {} on", self.git_ref);
        }
        eprintln!(
            "{}",
            say::error(format!("no machine could resolve {}", self.git_ref))
        );
        for failure in failures {
            eprintln!("    {failure}");
        }
        Ok(None)
    }
}

/// the working tree at `dir`, ready to upload
async fn pack(dir: &std::path::Path) -> Result<Snapshot> {
    let snapshot = Snapshot::capture(dir).await?;
    eprintln!(
        "{}",
        say::info(format!(
            "packed {} ({})",
            snapshot.label,
            format::bytes(snapshot.nar.len() as u64)
        ))
    );
    Ok(snapshot)
}
