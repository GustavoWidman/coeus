use super::CoeusClient;
use crate::{
    cli::main::ClientCommand,
    common::proto::*,
    utils::{log::Logger, plain_text},
};
use colored::Colorize;
use eyre::{Result, bail};
use log::{info, warn};
use std::time::Duration;

impl CoeusClient {
    pub async fn run_command(&mut self, command: ClientCommand) -> Result<()> {
        match command {
            ClientCommand::Identity
            | ClientCommand::Switch { .. }
            | ClientCommand::Reconcile { .. }
            | ClientCommand::Bootstrap { .. }
            | ClientCommand::ActivateLocal { .. }
            | ClientCommand::ReconcileLocal { .. }
            | ClientCommand::BootstrapLocal { .. } => {
                bail!("independent operator commands must not be sent through coeusd");
            }
            ClientCommand::Deploy(mut request) => {
                if request.request_id == 0 {
                    request.request_id = rand::random::<u64>().max(1);
                }
                // Emit and flush before sending: a dropped ACK must not hide the retry key.
                Logger::retry_id(request.request_id)?;
                match self.request(request).await? {
                    Packet::DeployAccepted(accepted) => {
                        info!(
                            "{} {} — {}",
                            "accepted".green(),
                            accepted.job_id,
                            plain_text::bounded(&accepted.message, 1024)
                        );
                        Ok(())
                    }
                    _ => {
                        bail!("unexpected deployment response");
                    }
                }
            }
            ClientCommand::Status { job_id, watch } => self.show_status(job_id, watch).await,
            ClientCommand::Logs {
                job_id,
                after,
                follow,
                metadata,
                source,
            } => {
                self.show_logs(job_id, after, follow, metadata, source)
                    .await
            }
            ClientCommand::Attach {
                job_id,
                live,
                after,
            } => self.attach(job_id, live, after).await,
            ClientCommand::Cancel { job_id } => match self.request(CancelRequest { job_id }).await?
            {
                Packet::StatusResponse(response) => {
                    print_status(&response.jobs);
                    Ok(())
                }
                _ => {
                    bail!("unexpected cancellation response");
                }
            },
        }
    }

    async fn show_status(&mut self, job_id: Option<JobId>, watch: bool) -> Result<()> {
        loop {
            let mut after = None;
            let mut finished = true;
            loop {
                let Packet::StatusResponse(response) =
                    self.request(StatusRequest { job_id, after }).await?
                else {
                    bail!("unexpected status response");
                };
                print_status(&response.jobs);
                finished &= response.jobs.iter().all(|job| job.phase.is_terminal());
                after = response.next_after;
                if after.is_none() {
                    break;
                }
            }
            if !watch || finished {
                return Ok(());
            }
            if stop_watching().await? {
                return Ok(());
            }
        }
    }

    async fn show_logs(
        &mut self,
        job_id: JobId,
        mut after: u64,
        follow: bool,
        metadata: bool,
        source: Option<crate::cli::main::LogFilter>,
    ) -> Result<()> {
        loop {
            let Packet::LogsResponse(response) =
                self.request(LogsRequest { job_id, after }).await?
            else {
                bail!("unexpected logs response");
            };
            if response.gap {
                warn!("older output was evicted");
            }
            for entry in &response.entries {
                let text = plain_text::bounded(&entry.text, 1024);
                if source.is_some_and(|filter| !filter.matches(entry.source)) {
                    continue;
                }
                if metadata {
                    let label = match entry.source {
                        LogSource::Lifecycle => "phase",
                        LogSource::Stdout => "stdout",
                        LogSource::Stderr => "stderr",
                        LogSource::Terminal => "terminal",
                    };
                    println!("{} [{}] {}", entry.cursor, label, text);
                } else {
                    println!("{text}");
                }
            }
            after = response.next_cursor;
            // Drain all retained pages, including after a terminal status.
            if !response.entries.is_empty() {
                continue;
            }
            if !follow || response.finished {
                return Ok(());
            }
            if stop_watching().await? {
                return Ok(());
            }
        }
    }
}

fn print_status(jobs: &[JobStatus]) {
    for job in jobs {
        let phase = format!("{:?}", job.phase);
        let phase = match job.phase {
            DeployPhase::Succeeded => phase.green(),
            DeployPhase::Failed | DeployPhase::Interrupted => phase.red(),
            DeployPhase::Cancelled => phase.yellow(),
            _ => phase.cyan(),
        };
        info!(
            "{} {} {phase}{}{}",
            job.job_id,
            plain_text::bounded(&job.revision, 256),
            if job.cancel_requested {
                " (cancellation requested)"
            } else {
                ""
            },
            job.message
                .as_ref()
                .map_or(String::new(), |message| format!(
                    " — {}",
                    plain_text::bounded(message, 1024)
                ))
        );
    }
}

async fn stop_watching() -> Result<bool> {
    tokio::select! {
        _ = tokio::time::sleep(Duration::from_millis(500)) => Ok(false),
        signal = tokio::signal::ctrl_c() => { signal?; Ok(true) },
    }
}
