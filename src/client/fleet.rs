//! CLI orchestration: resolve/authenticate before dispatch, never retry ambiguous ACKs.
use super::{CoeusClient, EndpointUnreachable, target::Target};
use crate::{
    cli::main::{ClientCommand, DeployArgs},
    common::proto::*,
    config::ClientConfig,
    utils::{log::Logger, plain_text},
};
use eyre::{Result, WrapErr, bail, ensure, eyre};
use futures_util::{StreamExt, stream};
use std::{
    collections::HashSet,
    io::{IsTerminal, Write},
    sync::Arc,
    time::Duration,
};
use tokio::sync::Mutex;

pub(super) struct AcceptedTarget {
    pub name: String,
    pub client: CoeusClient,
    pub job_id: JobId,
}

pub async fn run(config: ClientConfig, command: ClientCommand, no_tty: bool) -> Result<()> {
    let interactive = interactive(
        no_tty,
        std::io::stdin().is_terminal(),
        std::io::stdout().is_terminal(),
    );
    match command {
        ClientCommand::Deploy(args) => deploy(config, args, interactive).await,
        ClientCommand::Status { target, watch, job } => {
            if let Some(id) = job {
                let name = target
                    .as_deref()
                    .ok_or_else(|| eyre!("--job requires an explicit target"))?;
                return connect(&config, name)
                    .await?
                    .show_status(Some(id), watch)
                    .await;
            }
            let names = targets(&config, target.as_deref())?;
            loop {
                let results = stream::iter(names.iter().cloned())
                    .map(|name| {
                        let config = config.clone();
                        async move {
                            let result = async {
                                let mut client = connect(&config, &name).await?;
                                let Packet::MachineStatusResponse(status) =
                                    client.request(MachineStatusRequest {}).await?
                                else {
                                    bail!("unexpected machine status response");
                                };
                                Ok(status)
                            }
                            .await;
                            (name, result)
                        }
                    })
                    .buffer_unordered(8)
                    .collect::<Vec<_>>()
                    .await;
                println!("TARGET\tACTIVITY\tJOB\tREVISION\tOUTCOME");
                let mut failed = false;
                for (name, result) in results {
                    match result {
                        Ok(status) => {
                            println!("{}", status_row(&name, &status));
                            if target.is_some()
                                && let Some(job) = status.active.as_ref().or(status.latest.as_ref())
                            {
                                super::commands::print_status(std::slice::from_ref(job));
                            }
                        }
                        Err(error) => {
                            failed = true;
                            eprintln!("{}: {error}", plain_text::bounded(&name, 512));
                        }
                    }
                }
                if !watch {
                    ensure!(!failed, "one or more targets could not be queried");
                    return Ok(());
                }
                tokio::select! { _ = tokio::time::sleep(Duration::from_millis(500)) => {}, signal = tokio::signal::ctrl_c() => { signal?; return Ok(()) } }
            }
        }
        ClientCommand::Attach {
            target,
            live,
            after,
        } => {
            connect(&config, &target)
                .await?
                .attach_shared(live, after)
                .await
        }
        ClientCommand::Logs {
            job_id,
            target,
            after,
            follow,
            metadata,
            source,
        } => {
            let name = one_target(&config, target)?;
            connect(&config, &name)
                .await?
                .show_logs(job_id, after, follow, metadata, source)
                .await
        }
        ClientCommand::Cancel { job_id, target } => {
            let name = one_target(&config, target)?;
            let mut client = connect(&config, &name).await?;
            let Packet::StatusResponse(response) = client.request(CancelRequest { job_id }).await?
            else {
                bail!("unexpected cancel response");
            };
            super::commands::print_status(&response.jobs);
            Ok(())
        }
        _ => {
            bail!("independent operator command must not be sent to daemon");
        }
    }
}
fn interactive(no_tty: bool, input: bool, output: bool) -> bool {
    !no_tty && input && output
}
fn targets(config: &ClientConfig, target: Option<&str>) -> Result<Vec<String>> {
    if let Some(target) = target {
        Target::parse(target)?;
        return Ok(vec![target.into()]);
    }
    ensure!(
        !config.fleet.is_empty(),
        "no fleet configured; specify a target or settings.fleet"
    );
    let mut unique = HashSet::new();
    for name in &config.fleet {
        Target::parse(name)?;
        ensure!(unique.insert(name), "duplicate fleet target");
    }
    Ok(config.fleet.clone())
}
fn one_target(config: &ClientConfig, target: Option<String>) -> Result<String> {
    if let Some(target) = target {
        return Ok(target);
    }
    ensure!(
        config.fleet.len() == 1,
        "--target is required unless fleet contains exactly one machine"
    );
    Ok(config.fleet[0].clone())
}

pub(super) async fn connect(config: &ClientConfig, name: &str) -> Result<CoeusClient> {
    let target = Target::parse(name)?;
    let addresses = target
        .resolve()
        .await
        .wrap_err_with(|| format!("resolve {name}"))?;
    let mut last = None;
    for address in addresses {
        match CoeusClient::new(address, &config.key).await {
            Ok(client) => return Ok(client),
            Err(error) if error.downcast_ref::<EndpointUnreachable>().is_some() => {
                last = Some(error)
            }
            // Any live endpoint with authentication/protocol failure is definitive.
            Err(error) => {
                return Err(error.wrap_err(
                    "endpoint exists but authentication/hello failed; bootstrap forbidden",
                ));
            }
        }
    }
    Err(last.unwrap_or_else(|| eyre!("no endpoint addresses")))
}
async fn deploy(config: ClientConfig, args: DeployArgs, interactive: bool) -> Result<()> {
    let names = targets(&config, args.target.as_deref())?;
    ensure!(
        args.target.is_some() || args.request_id == 0,
        "explicit --request-id requires one target (fleet retries must be individually reviewed)"
    );
    let used = Arc::new(Mutex::new(HashSet::new()));
    let tasks = stream::iter(names)
        .map(|name| {
            let config = config.clone();
            let args = args.clone();
            let used = used.clone();
            async move {
                let result = dispatch(
                    &config,
                    &args,
                    &name,
                    interactive && args.target.is_some(),
                    used,
                )
                .await;
                (name, result)
            }
        })
        .buffer_unordered(8);
    let mut accepted = Vec::new();
    let mut errors = Vec::new();
    tokio::pin!(tasks);
    while let Some((name, result)) = tasks.next().await {
        match result {
            Ok(target) => accepted.push(target),
            Err(error) => {
                eprintln!("{}: {error:#}", plain_text::bounded(&name, 512));
                errors.push(name);
            }
        }
    }
    if interactive {
        if args.target.is_some() {
            if let Some(mut target) = accepted.pop() {
                target.client.attach(target.job_id, false, None).await?;
            }
        } else if !accepted.is_empty() {
            super::tui::follow(accepted, &errors).await?;
        }
    }
    ensure!(
        errors.is_empty(),
        "deployment dispatch failed for {} target(s); accepted deployments continue independently",
        errors.len()
    );
    Ok(())
}
async fn dispatch(
    config: &ClientConfig,
    args: &DeployArgs,
    name: &str,
    offer: bool,
    used: Arc<Mutex<HashSet<std::net::SocketAddr>>>,
) -> Result<AcceptedTarget> {
    let mut client = match connect(config, name).await {
        Ok(client) => client,
        Err(error) if offer && error.downcast_ref::<EndpointUnreachable>().is_some() => {
            bootstrap_offer(config, name).await?;
            connect(config, name).await?
        }
        Err(error) => return Err(error),
    };
    let address = client.stream.peer_addr()?;
    ensure!(
        used.lock().await.insert(address),
        "duplicate fleet endpoint; refusing second deployment"
    );
    let nonce = if args.request_id == 0 {
        rand::random::<u64>().max(1)
    } else {
        args.request_id
    };
    // Flush the machine label and nonce before sending, including quiet mode.
    {
        let mut out = std::io::stdout().lock();
        writeln!(out, "target: {}", plain_text::bounded(name, 512))?;
        out.flush()?;
    }
    Logger::retry_id(nonce)?;
    let request = DeployRequest {
        request_id: nonce,
        rev: args
            .rev
            .clone()
            .unwrap_or_else(|| config.default_revision.clone()),
        dry_run: args.dry_run,
        clean_substituters: args.clean_substituters,
    };
    let Packet::DeployAccepted(accepted) = client.request(request).await.wrap_err(
        "submission may have been accepted: query this target's status; do not automatically retry",
    )?
    else {
        bail!("unexpected deployment response; query status before retrying");
    };
    ensure!(
        accepted.job_id != 0,
        "invalid assigned job ID; query status before retrying"
    );
    println!(
        "{} accepted {} — {}",
        plain_text::bounded(name, 512),
        accepted.job_id,
        plain_text::bounded(&accepted.message, 1024)
    );
    Ok(AcceptedTarget {
        name: name.into(),
        client,
        job_id: accepted.job_id,
    })
}

async fn bootstrap_offer(config: &ClientConfig, name: &str) -> Result<()> {
    let settings = config.bootstrap.get(name).ok_or_else(|| eyre!("endpoint unreachable; bootstrap prerequisites not configured for this target. Use explicit coeus bootstrap --help (never auto-retry a deployment)"))?;
    eprintln!(
        "{name} unreachable. Bootstrap via verified SSH with existing configured prerequisites? [y/N]"
    );
    let answer = tokio::task::spawn_blocking(|| {
        let mut s = String::new();
        std::io::stdin().read_line(&mut s).map(|_| s)
    })
    .await??;
    ensure!(
        answer.trim().eq_ignore_ascii_case("y"),
        "bootstrap declined"
    );
    let target = Target::parse(name)?;
    let args = crate::activation::RemoteArgs {
        ssh: if target.is_local() {
            "127.0.0.1".into()
        } else {
            target.host
        },
        activation_directory: settings.activation_directory.clone(),
        peer_config: settings.peer_config.clone(),
        copy_binary: settings.copy_binary.clone(),
        operator_sudo: settings.operator_sudo,
    };
    crate::activation::run_remote(
        &args,
        rand::random::<u64>().max(1),
        Some((&settings.flake, &settings.hostname, &settings.nix_binary)),
        false,
    )
    .await
}

pub(super) fn status_row(name: &str, status: &MachineStatusResponse) -> String {
    let (activity, job) = match (&status.active, &status.latest) {
        (Some(job), _) => ("Active", Some(job)),
        (None, Some(job)) if !job.phase.is_terminal() => ("Pending", Some(job)),
        (None, Some(job)) => ("Completed", Some(job)),
        _ => ("Idle", None),
    };
    match job {
        Some(job) => format!(
            "{}\t{activity}\t{}\t{}\t{:?}{}",
            plain_text::bounded(name, 512),
            job.job_id,
            plain_text::bounded(&job.revision, 256),
            job.phase,
            if job.cancel_requested {
                " (cancel requested)"
            } else {
                ""
            }
        ),
        None => format!("{}\t{activity}\t-\t-\t-", plain_text::bounded(name, 512)),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tty_policy_and_completed_not_success() {
        for (no, input, output) in [
            (true, true, true),
            (false, false, true),
            (false, true, false),
        ] {
            assert!(!interactive(no, input, output));
        }
        assert!(interactive(false, true, true));
        let job = JobStatus {
            job_id: 7,
            revision: "main".into(),
            commit: None,
            phase: DeployPhase::Failed,
            cancel_requested: false,
            message: None,
        };
        let row = status_row(
            "lab",
            &MachineStatusResponse {
                active: None,
                latest: Some(job),
            },
        );
        assert!(row.contains("Completed\t7\tmain\tFailed"));
    }
}
