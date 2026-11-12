// the bootstrap's run unit fills this module in
#![allow(dead_code, unused_variables)]

use std::path::PathBuf;

use clap::Args;
use eyre::Result;

use crate::{
    bootstrap::Finale,
    cli::{Context, exit::Exit},
    common::model::Hostname,
};

#[derive(Args, Debug)]
pub struct BootstrapArgs {
    /// The machine's name in the fleet; without one, list the bootstraps in progress
    pub host: Option<Hostname>,

    /// How ssh reaches it while it runs its stock system, e.g. nixos@192.168.1.40
    #[arg(long, value_name = "DESTINATION")]
    pub to: Option<String>,

    /// Your checkout of the configuration repository [default: the saved one, else the current directory]
    #[arg(long, value_name = "DIR")]
    pub repo: Option<PathBuf>,

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
    #[arg(long, requires = "host", conflicts_with_all = ["to", "repo", "answers", "retry"])]
    pub forget: bool,
}

impl BootstrapArgs {
    pub async fn run(self, context: &Context) -> Result<Exit> {
        // TODO:
        // let Some(host) = self.host else { return list(StateDir::all()?) };
        // let state = StateDir::of(&host)?;
        // if self.forget { remove the session key if the machine is reachable,
        //     state.forget(); return Ok(Exit::Success) }
        // let mut bootstrap = Bootstrap { saved: state.load()? with --to/--repo over it, … };
        // let console = Console::start(context.live, Checklist::new(heading, last run))?;
        // let finale = bootstrap::run(&mut bootstrap, &console).await?;
        // record Saved::last_run; console.finish(finale); Ok(exit(&finale))
        todo!()
    }
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
