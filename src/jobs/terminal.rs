//! Volatile raw terminal replay, independent of network readers and durable logs.
use crate::common::proto::*;
use eyre::{Result, ensure};
#[cfg(unix)]
use std::fs::File;
use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
};

pub(super) const RETAIN_BYTES: usize = 256 * 1024;
pub(super) type Terminals = Arc<tokio::sync::Mutex<HashMap<JobId, Arc<Mutex<TerminalOutput>>>>>;

#[derive(Default)]
pub(crate) struct TerminalOutput {
    bytes: VecDeque<u8>,
    end: u64,
    size: TerminalSize,
    #[cfg(unix)]
    master: Option<Arc<File>>,
    #[cfg(unix)]
    group: Option<nix::unistd::Pid>,
}
impl TerminalOutput {
    pub(crate) fn push(&mut self, bytes: &[u8]) -> Result<()> {
        self.end = self
            .end
            .checked_add(bytes.len() as u64)
            .ok_or_else(|| eyre::eyre!("terminal cursor exhausted"))?;
        // A caller never supplies more than one read, but bound even oversized calls.
        if bytes.len() >= RETAIN_BYTES {
            self.bytes.clear();
            self.bytes.extend(&bytes[bytes.len() - RETAIN_BYTES..]);
        } else {
            let evict = (self.bytes.len() + bytes.len()).saturating_sub(RETAIN_BYTES);
            self.bytes.drain(..evict);
            self.bytes.extend(bytes);
        }
        Ok(())
    }
    pub(crate) fn page(
        &mut self,
        request: &TerminalRequest,
        finished: bool,
        error: Option<String>,
    ) -> Result<TerminalResponse> {
        self.resize(request.size.validate()?)?;
        let after = request.after.unwrap_or(self.end);
        ensure!(
            after <= self.end,
            "terminal cursor is beyond available output"
        );
        let start = self.end - self.bytes.len() as u64;
        let gap = after < start;
        let bytes: Vec<_> = if gap {
            Vec::new()
        } else {
            self.bytes
                .iter()
                .skip((after - start) as usize)
                .take(TERMINAL_PAGE_BYTES)
                .copied()
                .collect()
        };
        Ok(TerminalResponse {
            job_id: request.job_id,
            next_cursor: if gap {
                start
            } else {
                after + bytes.len() as u64
            },
            bytes,
            gap,
            available: true,
            finished,
            error,
        })
    }
    fn resize(&mut self, size: TerminalSize) -> Result<()> {
        #[cfg(unix)]
        if size != self.size
            && let Some(master) = &self.master
        {
            set_size(master, size)?;
            if let Some(group) = self.group {
                match nix::sys::signal::killpg(group, nix::sys::signal::Signal::SIGWINCH) {
                    Ok(()) | Err(nix::errno::Errno::ESRCH) => {}
                    Err(error) => return Err(error.into()),
                }
            }
        }
        self.size = size;
        Ok(())
    }
    #[cfg(unix)]
    pub(crate) fn register(&mut self, master: Arc<File>) -> Result<()> {
        set_size(&master, self.size)?;
        self.master = Some(master);
        Ok(())
    }
    #[cfg(unix)]
    pub(crate) fn process_group(&mut self, pid: u32) {
        self.group = Some(nix::unistd::Pid::from_raw(pid as i32));
    }
    #[cfg(unix)]
    pub(crate) fn unregister(&mut self) {
        self.master = None;
        self.group = None;
    }
}

impl super::DeploymentJobs {
    pub async fn terminal(&self, request: TerminalRequest) -> Result<TerminalResponse> {
        request.size.validate()?;
        // Retain the ledger guard through raw lookup/page generation so admission
        // cannot evict/reuse this ID between status lookup and terminal lookup.
        let ledger = self.ledger.lock().await;
        let status = &ledger.record(request.job_id)?.status;
        let finished = status.phase.is_terminal();
        let error = if finished && status.phase != DeployPhase::Succeeded {
            Some(
                status
                    .message
                    .clone()
                    .unwrap_or_else(|| format!("deployment {:?}", status.phase)),
            )
        } else {
            None
        };
        let terminals = self.terminals.lock().await;
        let Some(output) = terminals.get(&request.job_id) else {
            return Ok(TerminalResponse {
                job_id: request.job_id,
                bytes: vec![],
                next_cursor: 0,
                gap: false,
                available: false,
                finished,
                error,
            });
        };
        output
            .lock()
            .map_err(|_| eyre::eyre!("terminal state poisoned"))?
            .page(&request, finished, error)
    }
}

#[cfg(unix)]
fn set_size(master: &File, size: TerminalSize) -> Result<()> {
    rustix::termios::tcsetwinsize(
        master,
        rustix::termios::Winsize {
            ws_row: size.rows,
            ws_col: size.cols,
            ws_xpixel: 0,
            ws_ypixel: 0,
        },
    )?;
    Ok(())
}

#[cfg(test)]
#[path = "terminal_tests.rs"]
mod tests;
