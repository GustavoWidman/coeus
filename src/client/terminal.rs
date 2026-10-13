//! Output-only attachment: local Ctrl-C closes this view, never sends Cancel/SIGINT.
use super::CoeusClient;
use crate::common::proto::*;
use eyre::{Result, bail, ensure};
use std::time::Duration;

#[cfg(unix)]
impl CoeusClient {
    pub(super) async fn attach(
        &mut self,
        job_id: JobId,
        live: bool,
        after: Option<u64>,
    ) -> Result<()> {
        let mut cursor = if live { None } else { Some(after.unwrap_or(0)) };
        // Keep one receiver for the entire view: recreating ctrl_c futures per
        // page could lose SIGINT between a completed write and the next select.
        let mut interrupt =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
        let output = super::terminal_output::TerminalOutput::new()?;
        // No raw input mode, no stdin forwarding, no remote signal forwarding.
        loop {
            let response = tokio::select! {
                biased;
                _ = interrupt.recv() => return Ok(()),
                response = self.request(TerminalRequest { job_id, after: cursor, size: local_size() }) => response?,
            };
            let Packet::TerminalResponse(page) = response else {
                bail!("unexpected terminal response");
            };
            ensure!(
                page.job_id == job_id,
                "terminal response belongs to another job"
            );
            ensure!(
                page.bytes.len() <= TERMINAL_PAGE_BYTES,
                "oversized terminal page"
            );
            ensure!(
                page.available,
                "raw terminal history unavailable after daemon restart; use status/logs"
            );
            ensure!(
                !page.gap,
                "raw terminal history gap; detach and use attach --live to explicitly skip replay (terminal state cannot be reconstructed)"
            );
            if let Some(cursor) = cursor {
                ensure!(
                    page.next_cursor
                        == cursor
                            .checked_add(page.bytes.len() as u64)
                            .ok_or_else(|| eyre::eyre!("terminal cursor exhausted"))?,
                    "invalid terminal cursor"
                );
            } else {
                ensure!(
                    page.bytes.is_empty(),
                    "live attach returned historical output"
                );
            }
            // Never decode, sanitize, prefix or append a newline. Write failures detach.
            tokio::select! {
                biased;
                _ = interrupt.recv() => return Ok(()),
                result = output.write(&page.bytes) => result?,
            }

            cursor = Some(page.next_cursor);
            if !page.bytes.is_empty() {
                continue;
            }
            if page.finished {
                if let Some(error) = page.error {
                    bail!(
                        "deployment ended: {}",
                        crate::utils::plain_text::bounded(&error, 1024)
                    );
                }
                return Ok(());
            }
            tokio::select! {
                _ = interrupt.recv() => return Ok(()),
                _ = tokio::time::sleep(Duration::from_millis(50)) => {}
            }
        }
    }
}
fn local_size() -> TerminalSize {
    #[cfg(unix)]
    if let Ok(size) = rustix::termios::tcgetwinsize(std::io::stdout()) {
        let size = TerminalSize {
            rows: size.ws_row,
            cols: size.ws_col,
        };
        if size.validate().is_ok() {
            return size;
        }
    }
    TerminalSize::default()
}

#[cfg(not(unix))]
impl CoeusClient {
    pub(super) async fn attach(
        &mut self,
        _job_id: JobId,
        _live: bool,
        _after: Option<u64>,
    ) -> Result<()> {
        bail!("terminal attachment requires Unix");
    }
}
