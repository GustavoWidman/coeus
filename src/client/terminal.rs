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

#[cfg(unix)]
impl CoeusClient {
    pub(super) async fn attach_shared(&mut self, live: bool, after: Option<u64>) -> Result<()> {
        let mut cursor = if live { None } else { Some(after.unwrap_or(0)) };
        let mut interrupt =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
        let output = super::terminal_output::TerminalOutput::new()?;
        loop {
            let response = tokio::select! {
                biased;
                _ = interrupt.recv() => return Ok(()),
                response = self.request(SharedTerminalRequest { after: cursor, size: local_size() }) => response?,
            };
            let Packet::SharedTerminalResponse(page) = response else {
                bail!("unexpected shared terminal response");
            };
            validate_shared_page(cursor, &page)?;
            for chunk in &page.chunks {
                tokio::select! {
                    biased;
                    _ = interrupt.recv() => return Ok(()),
                    result = output.write(&chunk.bytes) => result?,
                }
            }
            cursor = Some(page.next_cursor);
            if page.chunks.iter().all(|chunk| chunk.bytes.is_empty()) {
                tokio::select! { _ = interrupt.recv() => return Ok(()), _ = tokio::time::sleep(Duration::from_millis(50)) => {} }
            }
        }
    }
}
fn validate_shared_page(cursor: Option<u64>, page: &SharedTerminalResponse) -> Result<()> {
    ensure!(
        !page.gap,
        "shared terminal history gap; explicitly reattach --live to skip lost output"
    );
    let bytes = page.chunks.iter().try_fold(0usize, |sum, chunk| {
        sum.checked_add(chunk.bytes.len())
            .ok_or_else(|| eyre::eyre!("shared page size overflow"))
    })?;
    ensure!(
        bytes <= TERMINAL_PAGE_BYTES,
        "oversized shared terminal page"
    );
    if let Some(cursor) = cursor {
        ensure!(
            cursor.checked_add(bytes as u64) == Some(page.next_cursor),
            "invalid shared terminal cursor"
        );
    } else {
        ensure!(bytes == 0, "live attach returned historical output");
    }
    Ok(())
}
#[cfg(not(unix))]
impl CoeusClient {
    pub(super) async fn attach_shared(&mut self, _live: bool, _after: Option<u64>) -> Result<()> {
        bail!("terminal attachment requires Unix");
    }
}
#[cfg(test)]
mod shared_tests {
    use super::*;
    #[test]
    fn validates_cross_job_cursor_and_rejects_loss_and_oversize() -> Result<()> {
        let mut page = SharedTerminalResponse {
            chunks: vec![
                SharedTerminalChunk {
                    job_id: 1,
                    bytes: b"a".to_vec(),
                },
                SharedTerminalChunk {
                    job_id: 2,
                    bytes: b"bc".to_vec(),
                },
            ],
            next_cursor: 3,
            gap: false,
        };
        validate_shared_page(Some(0), &page)?;
        assert!(validate_shared_page(None, &page).is_err());
        page.gap = true;
        assert!(validate_shared_page(Some(0), &page).is_err());
        page.gap = false;
        page.next_cursor = 4;
        assert!(validate_shared_page(Some(0), &page).is_err());
        Ok(())
    }
}
