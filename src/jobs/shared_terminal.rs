//! One bounded, daemon-lifetime producer feed. Readers never own execution or
//! consume one another's bytes. Per-job replay remains separate.
use crate::common::proto::*;
use eyre::{Result, ensure};
use std::collections::VecDeque;

#[derive(Default)]
pub(super) struct SharedOutput {
    chunks: VecDeque<SharedTerminalChunk>,
    retained: usize,
    end: u64,
}
impl SharedOutput {
    pub(super) fn push(&mut self, job_id: JobId, bytes: &[u8]) -> Result<()> {
        let end = self
            .end
            .checked_add(bytes.len() as u64)
            .ok_or_else(|| eyre::eyre!("shared terminal cursor exhausted"))?;
        if bytes.is_empty() {
            return Ok(());
        }
        let bytes = if bytes.len() > super::terminal::RETAIN_BYTES {
            &bytes[bytes.len() - super::terminal::RETAIN_BYTES..]
        } else {
            bytes
        };
        self.chunks.push_back(SharedTerminalChunk {
            job_id,
            bytes: bytes.to_vec(),
        });
        self.retained += bytes.len();
        while self.retained > super::terminal::RETAIN_BYTES {
            let excess = self.retained - super::terminal::RETAIN_BYTES;
            let Some(front) = self.chunks.front_mut() else {
                break;
            };
            let remove = excess.min(front.bytes.len());
            front.bytes.drain(..remove);
            self.retained -= remove;
            if front.bytes.is_empty() {
                self.chunks.pop_front();
            }
        }
        self.end = end;
        Ok(())
    }
    fn page(&self, after: Option<u64>) -> Result<SharedTerminalResponse> {
        let after = after.unwrap_or(self.end);
        ensure!(
            after <= self.end,
            "shared terminal cursor beyond daemon output; daemon may have restarted"
        );
        let start = self.end - self.retained as u64;
        if after < start {
            return Ok(SharedTerminalResponse {
                chunks: vec![],
                next_cursor: start,
                gap: true,
            });
        }
        let mut skip = (after - start) as usize;
        let mut remaining = TERMINAL_PAGE_BYTES;
        let mut chunks = Vec::new();
        for chunk in &self.chunks {
            if skip >= chunk.bytes.len() {
                skip -= chunk.bytes.len();
                continue;
            }
            let count = remaining.min(chunk.bytes.len() - skip);
            chunks.push(SharedTerminalChunk {
                job_id: chunk.job_id,
                bytes: chunk.bytes[skip..skip + count].to_vec(),
            });
            remaining -= count;
            skip = 0;
            if remaining == 0 {
                break;
            }
        }
        Ok(SharedTerminalResponse {
            chunks,
            next_cursor: after + (TERMINAL_PAGE_BYTES - remaining) as u64,
            gap: false,
        })
    }
}
impl super::DeploymentJobs {
    pub async fn shared_terminal(
        &self,
        request: SharedTerminalRequest,
    ) -> Result<SharedTerminalResponse> {
        request.size.validate()?;
        // Window size applies only to currently executing terminal, not historic jobs.
        let ledger = self.ledger.lock().await;
        if let Some(record) = ledger
            .records
            .iter()
            .find(|r| !r.status.phase.is_terminal() && r.status.phase != DeployPhase::Queued)
        {
            let terminals = self.terminals.lock().await;
            if let Some(output) = terminals.get(&record.status.job_id) {
                output
                    .lock()
                    .map_err(|_| eyre::eyre!("terminal state poisoned"))?
                    .resize(request.size)?;
            }
        }
        self.feed
            .lock()
            .map_err(|_| eyre::eyre!("shared terminal state poisoned"))?
            .page(request.after)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn independent_cursors_preserve_job_boundaries_and_ansi() -> Result<()> {
        let mut feed = SharedOutput::default();
        feed.push(1, b"\x1b[31mone\r")?;
        let first = feed.page(Some(0))?;
        feed.push(2, b"two\r\n")?;
        let second = feed.page(Some(first.next_cursor))?;
        assert_eq!(second.chunks[0].job_id, 2);
        assert_eq!(second.chunks[0].bytes, b"two\r\n");
        let all = feed.page(Some(0))?;
        assert_eq!(all.chunks.len(), 2);
        assert_eq!(all.chunks[0].bytes, b"\x1b[31mone\r");
        assert_eq!(feed.page(None)?.next_cursor, all.next_cursor);
        assert!(feed.page(None)?.chunks.is_empty());
        Ok(())
    }
    #[test]
    fn slow_readers_observe_gap_and_bounded_pages() -> Result<()> {
        let mut feed = SharedOutput::default();
        feed.push(1, &vec![b'x'; super::super::terminal::RETAIN_BYTES + 100])?;
        let gap = feed.page(Some(0))?;
        assert!(gap.gap);
        assert_eq!(gap.next_cursor, 100);
        let page = feed.page(Some(gap.next_cursor))?;
        assert_eq!(page.chunks[0].bytes.len(), TERMINAL_PAGE_BYTES);
        assert!(feed.page(Some(u64::MAX)).is_err());
        Ok(())
    }
}
