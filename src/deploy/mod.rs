mod command;
mod constants;
mod utils;

use colored::Colorize;
use eyre::Result;
use git2::{
    Cred, CredentialType, FetchOptions, RemoteCallbacks, Repository, build::CheckoutBuilder,
};
use log::debug;
use tempfile::TempDir;

use crate::{common::proto::DeployRequest, config::ServerConfig, deploy::command::CommandRunner};
use constants::{CLEAN_SUBSTITUTERS, CLEAN_TRUSTED_PUBLIC_KEYS, SYSTEM};

fn create_remote_with_local_config<'repo>(
    repo: &'repo Repository,
    name: &str,
    url: &str,
) -> std::result::Result<git2::Remote<'repo>, git2::Error> {
    let original_config = repo.config()?;
    let local_config = git2::Config::open(&repo.path().join("config"))?;

    repo.set_config(&local_config)?;
    let remote = repo.remote(name, url);
    repo.set_config(&original_config)?;

    remote
}

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
                let mut callbacks = RemoteCallbacks::new();
                callbacks.credentials(|_url, username, allowed_types| {
                    if allowed_types.contains(CredentialType::SSH_KEY) {
                        Cred::ssh_key_from_agent(username.unwrap_or("git"))
                    } else if allowed_types.contains(CredentialType::USERNAME) {
                        Cred::username(username.unwrap_or("git"))
                    } else {
                        Cred::default()
                    }
                });

                let mut fetch_options = FetchOptions::new();
                fetch_options.remote_callbacks(callbacks);

                let mut builder = git2::build::RepoBuilder::new();
                builder.fetch_options(fetch_options);
                builder.remote_create(create_remote_with_local_config);
                let repo = builder.clone(&url, &path)?;

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_remote_without_composite_url_rewrite() {
        let repo_dir = tempfile::tempdir().unwrap();
        let repo = Repository::init(repo_dir.path()).unwrap();
        let global_config = repo_dir.path().join("global.gitconfig");
        std::fs::write(
            &global_config,
            "[init]\n\tdefaultBranch = main\n[url \"git@example.com:\"]\n\tinsteadOf = https://example.com/\n",
        )
        .unwrap();

        let mut config = git2::Config::new().unwrap();
        config
            .add_file(&global_config, git2::ConfigLevel::Global, false)
            .unwrap();
        config
            .add_file(&repo.path().join("config"), git2::ConfigLevel::Local, false)
            .unwrap();
        repo.set_config(&config).unwrap();
        repo.set_head("refs/heads/main").unwrap();

        let url = "https://example.com/owner/repo.git";
        let remote = create_remote_with_local_config(&repo, "origin", url).unwrap();

        assert_eq!(remote.url(), Ok(url));
        assert_eq!(
            repo.config()
                .unwrap()
                .get_string("url.git@example.com:.insteadOf"),
            Ok("https://example.com/".to_owned())
        );
        assert!(
            repo.config()
                .unwrap()
                .get_string(&format!("url.{url}.insteadOf"))
                .is_err()
        );
        assert!(repo.is_empty().unwrap());
    }
}
