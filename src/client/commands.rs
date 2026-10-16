use super::{
    CoeusClient,
    presentation::{self, Interrupt, Style, Watch},
};
use crate::{common::proto::*, utils::plain_text};
use eyre::{Result, bail};
use std::time::Duration;

impl CoeusClient {
    pub(super) async fn show_status(&mut self, job_id: Option<JobId>, watch: bool) -> Result<()> {
        let mut display = Watch::new(watch)?;
        let mut interrupt = Interrupt::new()?;
        loop {
            let mut after = None;
            let mut jobs = Vec::new();
            loop {
                let response = if watch {
                    tokio::select! { biased; signal = interrupt.recv() => { signal?; return Ok(()); }, result = self.request(StatusRequest { job_id, after }) => result? }
                } else {
                    self.request(StatusRequest { job_id, after }).await?
                };
                let Packet::StatusResponse(response) = response else {
                    bail!("unexpected status response");
                };
                jobs.extend(response.jobs);
                after = response.next_after;
                if after.is_none() {
                    break;
                }
            }
            let text = presentation::job_details(&jobs, Style::stdout());
            tokio::select! { biased; signal = interrupt.recv(), if watch => { signal?; return Ok(()); }, result = display.emit(&text) => result? }
            if !watch || jobs.iter().all(|job| job.phase.is_terminal()) {
                return Ok(());
            }
            if stop_watching(&mut interrupt).await? {
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
        let mut warned_gap = false;
        let mut interrupt = Interrupt::new()?;
        loop {
            let response = if follow {
                tokio::select! { biased; signal = interrupt.recv() => { signal?; return Ok(()); }, result = self.request(LogsRequest { job_id, after }) => result? }
            } else {
                self.request(LogsRequest { job_id, after }).await?
            };
            let Packet::LogsResponse(response) = response else {
                bail!("unexpected logs response");
            };
            if response.gap && !warned_gap {
                eprintln!("Job {job_id}: older output is no longer retained.");
                warned_gap = true;
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
                    println!("{} [{label}] {text}", entry.cursor);
                } else {
                    println!("{text}");
                }
            }
            after = response.next_cursor;
            // Drain retained pages before stopping, even after a terminal result.
            if !response.entries.is_empty() {
                continue;
            }
            if !follow || response.finished {
                return Ok(());
            }
            if stop_watching(&mut interrupt).await? {
                return Ok(());
            }
        }
    }
}

pub(super) fn print_status(jobs: &[JobStatus]) {
    print!("{}", presentation::job_details(jobs, Style::stdout()));
}
async fn stop_watching(interrupt: &mut Interrupt) -> Result<bool> {
    tokio::select! {
        _ = tokio::time::sleep(Duration::from_millis(500)) => Ok(false),
        signal = interrupt.recv() => { signal?; Ok(true) },
    }
}
