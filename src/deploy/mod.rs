mod command;
mod constants;
mod utils;

use colored::Colorize;
use eyre::Result;
use git2::{Repository, build::CheckoutBuilder};
use log::debug;
use tempfile::TempDir;

use crate::{common::proto::DeployRequest, config::ServerConfig, deploy::command::CommandRunner};
use constants::{CLEAN_SUBSTITUTERS, CLEAN_TRUSTED_PUBLIC_KEYS, SYSTEM};

pub struct Deployer {
    config: ServerConfig,
}

impl Deployer {
    pub async fn new(config: ServerConfig) -> Result<Self> {
        Ok(Self { config })
    }

    pub async fn checkout(&self, rev: String) -> eyre::Result<TempDir> {
        let dir = tempfile::tempdir()?;
        debug!(
            "cloning repo {} into {}",
            self.config.repo_url.blue(),
            dir.path().display().to_string().purple()
        );

        tokio::task::spawn_blocking({
            let url = self.config.repo_url.clone();
            let path = dir.path().to_path_buf();

            move || -> eyre::Result<Repository> {
                let repo = Repository::clone(&url, &path)?;

                repo.checkout_tree(
                    &repo
                        .revparse_single(&rev)
                        .or_else(|_| repo.revparse_single(&format!("origin/{rev}")))?,
                    Some(CheckoutBuilder::new().force()),
                )?;

                debug!(
                    "checked out revision {} in {}",
                    rev.magenta(),
                    path.display().to_string().purple()
                );

                Ok(repo)
            }
        })
        .await??;

        Ok(dir)
    }

    pub async fn deploy(&self, options: &DeployRequest) -> Result<()> {
        debug!("deploying with options:\n{:?}", options);

        let repo = self.checkout(options.rev.clone()).await?;

        let mut runner = CommandRunner::new("nh")
            .path(repo.path())
            .args([SYSTEM, "build", "."])
            .arg("--quiet");

        // TODO detect when running in daemon mode
        if false {
            // depends on the sudoers configuration defined in the flake for coeusd
            runner = runner.args(["--elevation-strategy", "passwordless"]);
        }

        runner = runner
            .arg("--")
            .args(["--extra-experimental-features", "pipe-operators"])
            .args(["--option", "accept-flake-config", "true"]);

        // let action = match (options.boot || options.initial) && SYSTEM == "os" {
        //     true => "boot",
        //     false => "switch",
        // };
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
        runner.clone().call().await?;

        runner
            .replace_arg_at(1, action) // replace "build" with "switch" or "boot"
            .call()
            .await?;

        // if !options.apply_only {
        //     runner.clone().call().await?;
        // }

        // if !options.build_only {
        //     runner
        //         .replace_arg_at(1, action) // replace "build" with "switch" or "boot"
        //         .call()
        //         .await?;
        // }

        drop(repo); // only cleanup the temporary directory after the deployment is complete

        Ok(())
    }
}
