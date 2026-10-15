pub(crate) mod build;
pub(crate) mod command;
/// Command execution boundary used by the deployer and injected executor fixtures.
pub use command::CommandRunner;
mod constants;
mod output;
pub(crate) mod process;
#[cfg(unix)]
mod pty;

use eyre::Result;
use log::debug;

use crate::jobs::{DeploymentExecutor, JobContext};
use crate::{
    common::proto::{DeployPhase, DeployRequest},
    config::ServerConfig,
};
mod checkout;
use constants::{CLEAN_SUBSTITUTERS, CLEAN_TRUSTED_PUBLIC_KEYS};
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

        let directory = context.activation_directory().await?;
        let directory_copy = directory.clone();
        let store = tokio::task::spawn_blocking(move || {
            crate::activation::ReceiptStore::open(&directory_copy)
        })
        .await??;
        let root = store.root(context.job_id());
        eyre::ensure!(
            std::fs::symlink_metadata(&root).is_err(),
            "retained build already exists; inspect receipt, do not replay"
        );
        let current = tokio::task::spawn_blocking(|| {
            crate::activation::ServiceIdentity::read(std::path::Path::new("/run/current-system"))
        })
        .await??;
        let attribute = crate::activation::build_attribute(&current);
        let mut runner = CommandRunner::new("nix")
            .path(repo.directory.path())
            .args(build::flake_build_args(&attribute, &root));
        if options.clean_substituters {
            runner = runner
                .args(["--option", "substituters", CLEAN_SUBSTITUTERS])
                .args(["--option", "extra-substituters", ""])
                .args(["--option", "trusted-public-keys", CLEAN_TRUSTED_PUBLIC_KEYS])
                .args(["--option", "extra-trusted-public-keys", ""]);
        }
        context.phase(DeployPhase::Build).await?;
        runner.call_with_output(context).await?;
        context.check_cancelled()?;
        let id = context.job_id();
        let receipt = tokio::task::spawn_blocking(move || {
            let expected = crate::activation::ServiceIdentity::read(&root)?;
            crate::activation::prepare(
                &store,
                id,
                expected,
                Some(current),
                Some(std::process::id()),
            )
        })
        .await??;
        if options.dry_run {
            context.revoke_build_only().await?;
            context
                .log(
                    crate::common::proto::LogSource::Lifecycle,
                    "build-only: no activation performed; exact GC-rooted closure retained",
                )
                .await;
            return Ok(());
        }
        if receipt.manual {
            context.handoff().await?;
        } else {
            context.phase(DeployPhase::Switch).await?;
            let mut address = self.config.address();
            if address.ip().is_unspecified() {
                address.set_ip(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST));
            }
            let executor = crate::activation::NativeExecutor {
                address,
                key: self.config.key,
                operator_sudo: false,
                output: Some(context.clone()),
            };
            let activation_store =
                crate::activation::ReceiptStore::open_async(directory.clone()).await?;
            let result = tokio::select! {
                result = crate::activation::activate(&activation_store, id, &executor) => result,
                _ = context.cancelled() => { eyre::bail!("activation interrupted; reconcile retained receipt before continuing"); },
            };
            result?;
        }

        drop(repo); // only cleanup the temporary directory after the deployment is complete

        Ok(())
    }
}

impl DeploymentExecutor for Deployer {
    fn execute(&self, request: DeployRequest, context: JobContext) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move { self.deploy(&request, &context).await })
    }
}
