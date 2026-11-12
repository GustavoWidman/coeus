//! a job's log, its single record of what happened. one line per record:
//!
//! ```text
//! @nix {"action":"start",...}                       nix's internal-json, verbatim
//! @nix {"action":"msg","level":3,"msg":"<text>"}    any other process output, wrapped
//! @coeus {"at":1794443502123,"event":{...}}         a JobEvent
//! ```
//!
//! there is at most one writer at a time, enforced by an exclusive lock on
//! the file that [LogWriter] holds for its life. a runner holds it from start
//! to finish; coeusd takes it only for jobs without a live runner. a crashed
//! writer's lock goes away with its process, so a free lock on an unfinished
//! log means its runner is gone.

mod reader;
mod writer;

use std::{
    fs::{File, OpenOptions, TryLockError},
    path::PathBuf,
    thread,
    time::Duration,
};

use eyre::Result;

pub use reader::{LogItem, LogReader};
pub use writer::LogWriter;

#[derive(Debug, Clone)]
pub struct JobLog {
    path: PathBuf,
}

impl JobLog {
    pub const NIX: &'static str = "@nix ";
    pub const COEUS: &'static str = "@coeus ";
    /// no line is longer, so a reader can always hand out whole lines
    pub const MAX_LINE: usize = 32 * 1024;

    pub const fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn exists(&self) -> bool {
        self.path.exists()
    }

    /// takes the writer, or `None` when another process holds it.
    ///
    /// a holder only probing liveness keeps it for microseconds, so a few
    /// retries tell such a probe apart from a live runner.
    pub fn writer(&self) -> Result<Option<LogWriter>> {
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;

        for attempt in 0..20 {
            match file.try_lock() {
                Ok(()) => return Ok(Some(LogWriter::new(file))),
                Err(TryLockError::WouldBlock) if attempt < 19 => {
                    thread::sleep(Duration::from_millis(50))
                }
                Err(TryLockError::WouldBlock) => return Ok(None),
                Err(TryLockError::Error(error)) => return Err(error.into()),
            }
        }

        Ok(None)
    }

    /// whether a live process holds the writer, without keeping it
    pub fn is_held(&self) -> Result<bool> {
        let Ok(file) = File::open(&self.path) else {
            return Ok(false);
        };

        match file.try_lock() {
            Ok(()) => Ok(false),
            Err(TryLockError::WouldBlock) => Ok(true),
            Err(TryLockError::Error(error)) => Err(error.into()),
        }
    }

    pub async fn reader(&self, from: u64) -> Result<LogReader> {
        LogReader::open(self, from).await
    }
}
