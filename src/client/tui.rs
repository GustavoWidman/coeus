//! Fleet-only real-time view. Output guard restores alternate screen on every exit.
use super::fleet::AcceptedTarget;
use crate::{common::proto::*, utils::plain_text};
use eyre::{Result, bail};
use futures_util::{StreamExt, stream};
use std::time::Duration;

struct Row {
    target: AcceptedTarget,
    status: Option<JobStatus>,
    error: Option<String>,
}
impl Row {
    fn done(&self) -> bool {
        self.error.is_some()
            || self
                .status
                .as_ref()
                .is_some_and(|job| job.phase.is_terminal())
    }
}

#[cfg(unix)]
struct Screen(super::terminal_output::TerminalOutput);
#[cfg(unix)]
impl Drop for Screen {
    fn drop(&mut self) {
        self.0.cleanup(b"\x1b[0m\x1b[?25h\x1b[?1049l");
    }
}
#[cfg(unix)]
pub(super) async fn follow(targets: Vec<AcceptedTarget>, failures: &[String]) -> Result<()> {
    let output = Screen(super::terminal_output::TerminalOutput::new()?);
    let mut interrupt = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
    tokio::select! {
        _ = interrupt.recv() => return Ok(()),
        result = output.0.write(b"\x1b[?1049h\x1b[?25l") => result?,
    }
    let mut rows: Vec<_> = targets
        .into_iter()
        .map(|target| Row {
            target,
            status: None,
            error: None,
        })
        .collect();
    loop {
        let refresh = stream::iter(rows.iter_mut().filter(|row| !row.done()))
            .map(|row| async move {
                let result = async {
                    let Packet::StatusResponse(response) = row
                        .target
                        .client
                        .request(StatusRequest {
                            job_id: Some(row.target.job_id),
                            after: None,
                        })
                        .await?
                    else {
                        bail!("unexpected job status response");
                    };
                    let job = response
                        .jobs
                        .into_iter()
                        .find(|job| job.job_id == row.target.job_id)
                        .ok_or_else(|| eyre::eyre!("accepted job missing from status"))?;
                    Ok::<_, eyre::Report>(job)
                }
                .await;
                match result {
                    Ok(job) => row.status = Some(job),
                    Err(error) => row.error = Some(plain_text::bounded(&error.to_string(), 256)),
                }
            })
            .buffer_unordered(8)
            .collect::<Vec<_>>();
        tokio::select! { biased; _ = interrupt.recv() => return Ok(()), _ = refresh => {} }
        let frame = render(&rows, failures);
        tokio::select! { biased; _ = interrupt.recv() => return Ok(()), result = output.0.write(frame.as_bytes()) => result? }
        if rows.iter().all(Row::done) {
            drop(output);
            // Leave a useful durable summary after closing the alternate screen.
            for row in &rows {
                println!(
                    "{} job {}: {}",
                    plain_text::bounded(&row.target.name, 512),
                    row.target.job_id,
                    outcome(row)
                );
            }
            if rows.iter().any(|row| row.error.is_some()) {
                bail!(
                    "fleet observation disconnected; deployments may continue, use status/attach"
                );
            }
            if rows.iter().any(|row| {
                row.status
                    .as_ref()
                    .is_none_or(|job| job.phase != DeployPhase::Succeeded)
            }) {
                bail!(
                    "one or more observed deployments ended without success; inspect status/logs"
                );
            }
            return Ok(());
        }
        tokio::select! { _ = interrupt.recv() => return Ok(()), _ = tokio::time::sleep(Duration::from_millis(250)) => {} }
    }
}
fn outcome(row: &Row) -> String {
    if let Some(error) = &row.error {
        format!("Disconnected: {error}")
    } else if let Some(job) = &row.status {
        format!(
            "{:?}{}",
            job.phase,
            if job.cancel_requested {
                " (cancel requested)"
            } else {
                ""
            }
        )
    } else {
        "Accepted".into()
    }
}
fn render(rows: &[Row], failures: &[String]) -> String {
    // Explicit CRLF because TerminalOutput disables OPOST. All remote text sanitized.
    let mut frame = "\x1b[H\x1b[2JCoeus fleet — Ctrl+C detaches; deployments continue\r\nTARGET\tJOB\tPHASE\r\n".to_string();
    for row in rows {
        frame.push_str(&format!(
            "{}\t{}\t{}\r\n",
            plain_text::bounded(&row.target.name, 128),
            row.target.job_id,
            outcome(row)
        ));
    }
    for name in failures {
        frame.push_str(&format!(
            "{}\t-\tDispatch failed\r\n",
            plain_text::bounded(name, 128)
        ));
    }
    frame
}
#[cfg(not(unix))]
pub(super) async fn follow(_targets: Vec<AcceptedTarget>, _failures: &[String]) -> Result<()> {
    bail!("fleet terminal view requires Unix; use --no-tty");
}
