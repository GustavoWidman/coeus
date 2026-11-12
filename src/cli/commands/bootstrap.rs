use std::path::{Path, PathBuf};

use clap::Args;
use colored::Colorize;
use eyre::{Result, WrapErr};

use crate::{
    bootstrap::{
        self, Bootstrap, Finale, Options,
        branch::Branch,
        contract::Answers,
        remote::SessionKey,
        state::{LastRun, Saved, StateDir},
        world::{Gh, Live, Machine},
    },
    cli::{Console, Context, exit::Exit},
    common::model::{Hostname, Timestamp},
    ui::{
        Theme,
        checklist::{self, Checklist},
        format, say,
        text::pad,
    },
};

#[derive(Args, Debug)]
#[command(after_help = AFTER_HELP)]
pub struct BootstrapArgs {
    /// The machine's name in the fleet; without one, list the bootstraps in progress
    pub host: Option<Hostname>,

    /// How ssh reaches it while it runs its stock system, e.g. nixos@192.168.1.40
    #[arg(long, value_name = "DESTINATION")]
    pub to: Option<String>,

    /// Your checkout of the configuration repository [default: the saved one, else the current directory]
    #[arg(long, value_name = "DIR")]
    pub repo: Option<PathBuf>,

    /// The branch of origin to enroll the machine onto and open the pull request against [default: the saved one, else origin's default branch]
    #[arg(long, value_name = "BRANCH")]
    pub base: Option<String>,

    /// Answer one of the repository's questions ahead of time
    #[arg(long = "answer", value_name = "ID=VALUE")]
    pub answers: Vec<String>,

    /// The identity that opens every secret [default: the repository's]
    #[arg(long, value_name = "FILE")]
    pub identity: Option<PathBuf>,

    /// Trust the host key with this fingerprint without asking
    #[arg(long, value_name = "SHA256:…")]
    pub host_key: Option<String>,

    /// Order the first switch of this commit again after it failed
    #[arg(long)]
    pub retry: bool,

    /// Forget this machine's saved inputs, clone and session key; the branch and pull request stay
    #[arg(long, requires = "host", conflicts_with_all = ["to", "repo", "base", "answers", "retry"])]
    pub forget: bool,
}

const AFTER_HELP: &str = "\
Run it on an admin machine, one whose identity opens every secret. It reaches \
the machine over ssh, installs Determinate Nix on a Mac that lacks it, enrolls \
the machine on coeus/bootstrap/<host> in a private clone of the repository, \
rekeys the secrets it must open, pushes the branch with a draft pull request, \
switches the machine with the coeus its configuration pins, waits for its \
coeusd on the tailnet and marks the pull request ready.

Stop it whenever you like; running it again picks up from what it finds.

Exit status: 0 in the fleet, 1 failed or waiting on you, 2 unreached, 130 detached.";

impl BootstrapArgs {
    pub async fn run(self, context: &Context) -> Result<Exit> {
        let Some(host) = self.host.clone() else {
            return list();
        };
        let state = StateDir::of(&host)?;
        if self.forget {
            return forget(state, context).await;
        }

        let mut saved = state.load()?;
        self.apply(&mut saved)?;
        let options = Options {
            identity: self.identity,
            host_key: self.host_key,
            retry: self.retry,
            answers: Answers::given(&self.answers)?,
        };
        let checklist = Checklist::new(
            checklist::title(&host, saved.destination.as_deref()),
            subtitle(&host, &saved),
        );
        let mut bootstrap = Bootstrap {
            host,
            settings: context.settings.clone(),
            state,
            saved,
            options,
        };

        let console = Console::start(context.live, checklist)?;
        let finale = bootstrap::run(&mut bootstrap, &Live::default(), &Gh, &console).await?;
        console.finish(&bootstrap.state, &finale).await?;
        Ok(exit(&finale))
    }

    /// this run's arguments over what earlier runs saved
    fn apply(&self, saved: &mut Saved) -> Result<()> {
        if let Some(to) = &self.to {
            saved.destination = Some(to.clone());
        }
        let repo = match (&self.repo, &saved.repo) {
            (Some(repo), _) => Some(repo.clone()),
            (None, Some(_)) => None,
            (None, None) => Some(std::env::current_dir()?),
        };
        if let Some(repo) = repo {
            let repo = repo
                .canonicalize()
                .wrap_err_with(|| format!("no repository at {}", repo.display()))?;
            saved.repo = Some(repo);
        }
        if let Some(base) = &self.base {
            saved.base = Some(base.clone());
        }
        Ok(())
    }
}

/// `~/GitHub/nix · coeus/bootstrap/nova onto main`, and how the last run
/// ended
fn subtitle(host: &Hostname, saved: &Saved) -> Vec<String> {
    let mut line = Branch::name(host);
    if let Some(repo) = &saved.repo {
        line = format!("{} · {line}", shown(repo));
    }
    if let Some(base) = &saved.base {
        line = format!("{line} onto {base}");
    }
    let mut lines = vec![line];
    lines.extend(
        saved
            .last_run
            .as_ref()
            .map(|last| format!("the last run {}", last_run(last))),
    );
    lines
}

/// `detached during switch 14m ago`
fn last_run(last: &LastRun) -> String {
    let now = Timestamp::now();
    match &last.ended {
        Some(ended) => format!("ended {}, {}", ended.how, format::ago(ended.at, now)),
        None => format!(
            "started {} and never said how it ended",
            format::ago(last.started_at, now)
        ),
    }
}

/// every bootstrap this admin has in progress, and how each last ended
fn list() -> Result<Exit> {
    let all = StateDir::all()?;
    if all.is_empty() {
        println!("{}", say::info("no bootstrap in progress"));
        return Ok(Exit::Success);
    }
    let width = all
        .iter()
        .map(|(host, _)| host.as_str().len())
        .max()
        .unwrap_or(0);
    for (host, saved) in all {
        let running = StateDir::of(&host)?.locked()?;
        let how = match (&saved.last_run, running) {
            (_, true) => "running now".bold().to_string(),
            (Some(last), false) => last_run(last),
            (None, false) => "never ran".to_owned(),
        };
        let destination = saved.destination.unwrap_or_default();
        println!(
            "{}  {}  {how}",
            pad(&Theme::host(host.as_str()).to_string(), width),
            destination.dimmed()
        );
    }
    Ok(Exit::Success)
}

/// the session key out of the machine when it can still be reached, then
/// everything this admin kept of the bootstrap
async fn forget(state: StateDir, context: &Context) -> Result<Exit> {
    let host = state.host().clone();
    let lock = state.lock().await?;
    if SessionKey::existing(&state)?.is_some() {
        let saved = state.load()?;
        let checklist = Checklist::new(
            checklist::title(&host, saved.destination.as_deref()),
            Vec::new(),
        );
        let bootstrap = Bootstrap {
            host: host.clone(),
            settings: context.settings.clone(),
            state,
            saved,
            options: Options::default(),
        };
        let console = Console::start(false, checklist)?;
        let machine = Live::default();
        let removed = async {
            machine.reach(&bootstrap, &console).await?;
            machine.cleanup(&bootstrap, &console).await
        }
        .await;
        if let Err(error) = removed {
            println!(
                "{}",
                say::warn(format!(
                    "the session key stays in root's authorized_keys on {host} as \
                     coeus-bootstrap@{host}: {error:#}"
                ))
            );
        }
        drop(console);
        drop(lock);
        bootstrap.state.forget().await?;
    } else {
        drop(lock);
        state.forget().await?;
    }
    println!(
        "{}",
        say::success(format!(
            "forgot {host}; {} and its pull request stay",
            Branch::name(&host)
        ))
    );
    Ok(Exit::Success)
}

/// 0 in the fleet, 1 failed or blocked, 2 unreached, 130 detached
fn exit(finale: &Finale) -> Exit {
    match finale {
        Finale::Done(_) => Exit::Success,
        Finale::Failed { .. } | Finale::Blocked { .. } => Exit::Failed,
        Finale::Unreached { .. } => Exit::Unreached,
        Finale::Detached { .. } => Exit::Detached,
    }
}

/// a path under the home directory as `~/…`
fn shown(path: &Path) -> String {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    match home
        .as_deref()
        .and_then(|home| path.strip_prefix(home).ok())
    {
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}
