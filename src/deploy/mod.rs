pub(crate) mod command;
mod constants;
mod output;
mod process;

use colored::Colorize;
use eyre::Result;
use log::debug;

use crate::jobs::{DeploymentExecutor, JobContext};
use crate::{
    common::proto::{DeployPhase, DeployRequest},
    config::ServerConfig,
    deploy::command::CommandRunner,
};
mod checkout;
use constants::{CLEAN_SUBSTITUTERS, CLEAN_TRUSTED_PUBLIC_KEYS, SYSTEM};
use futures_util::future::BoxFuture;

pub struct Deployer {
    config: ServerConfig,
}

impl Deployer {
    pub async fn new(config: ServerConfig) -> Result<Self> {
        Ok(Self { config })
    }

    pub async fn deploy(&self, options: &DeployRequest, context: &JobContext) -> Result<()> {
        debug!("deploying with options:\n{:?}", options);

        context.phase(DeployPhase::Checkout).await?;
        // libgit2 is blocking: finish/join checkout before dropping its TempDir.
        // Cancellation is observed at the boundary, never by detaching the clone.
        let repo = self
            .checkout(options.rev.clone(), context.cancellation_token())
            .await?;
        context.checked_out(repo.commit.clone()).await?;
        context.check_cancelled()?;

        let mut runner = CommandRunner::new("nh")
            .path(repo.directory.path())
            .args([SYSTEM, "build", "."])
            .arg("--quiet");

        runner = runner
            .arg("--")
            .args(["--extra-experimental-features", "pipe-operators"])
            .args(["--option", "accept-flake-config", "true"]);

        let action = "switch";

        if options.clean_substituters {
            runner = runner
                .args(["--option", "substituters", CLEAN_SUBSTITUTERS])
                .args(["--option", "extra-substituters", ""])
                .args(["--option", "trusted-public-keys", CLEAN_TRUSTED_PUBLIC_KEYS])
                .args(["--option", "extra-trusted-public-keys", ""]);
        }

        if options.dry_run {
            runner = runner.args(["--option", "eval-cache", "false"]);
        }

        debug!(
            "running deployment command: {}",
            format!("{:?}", runner).blue()
        );

        // built-then-switch pattern
        context.phase(DeployPhase::Build).await?;
        runner.clone().call_with_output(context).await?;

        context.phase(DeployPhase::Switch).await?;
        runner
            .replace_arg_at(1, action) // replace "build" with "switch" or "boot"
            .call_with_output(context)
            .await?;

        drop(repo); // only cleanup the temporary directory after the deployment is complete

        Ok(())
    }
}

impl DeploymentExecutor for Deployer {
    fn execute(&self, request: DeployRequest, context: JobContext) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move { self.deploy(&request, &context).await })
    }
}
