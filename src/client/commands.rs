use super::CoeusClient;
use crate::{common::proto::*, utils::plain_text};
use colored::Colorize;
use eyre::{Result, bail};
use log::warn;
use std::time::Duration;

impl CoeusClient {
    pub(super) async fn show_status(&mut self, job_id: Option<JobId>, watch: bool) -> Result<()> {
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

    pub(super) async fn show_logs(
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

pub(super) fn print_status(jobs: &[JobStatus]) {
    for job in jobs {
        let phase = format!("{:?}", job.phase);
        let phase = match job.phase {
            DeployPhase::Succeeded => phase.green(),
            DeployPhase::Failed | DeployPhase::Interrupted => phase.red(),
            DeployPhase::Cancelled => phase.yellow(),
            _ => phase.cyan(),
        };
        println!(
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
