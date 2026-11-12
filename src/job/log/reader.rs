use eyre::Result;
use log::debug;
use tokio::{
    fs::File,
    io::{AsyncReadExt, AsyncSeekExt, SeekFrom},
};

use super::JobLog;
use crate::common::model::{JobEvent, Stamped};

/// one record of a job log
#[derive(Debug, PartialEq, Eq)]
pub enum LogItem {
    /// one or more whole lines of nix's internal-json
    Nix(Vec<u8>),
    Event(Stamped<JobEvent>),
}

/// reads a job log from a byte offset with its own cursor. it only ever
/// consumes whole lines: the writer may be mid-append, so a partial last line
/// waits for the next call.
pub struct LogReader {
    file: File,
    /// the file position of `buffer[0]`
    offset: u64,
    buffer: Vec<u8>,
}

impl LogReader {
    const READ: usize = 64 * 1024;

    pub(super) async fn open(log: &JobLog, from: u64) -> Result<Self> {
        let mut file = File::open(&log.path).await?;
        file.seek(SeekFrom::Start(from)).await?;

        Ok(Self {
            file,
            offset: from,
            buffer: Vec::with_capacity(Self::READ),
        })
    }

    /// where the next item starts, the resume point after a reconnect
    pub const fn offset(&self) -> u64 {
        self.offset
    }

    /// the next record, or `None` at the current end of the log. consecutive
    /// nix lines come back as one item of at most `max_nix` bytes, unless a
    /// single line is longer.
    pub async fn next(&mut self, max_nix: usize) -> Result<Option<LogItem>> {
        loop {
            let Some(end) = self.line_end(0).await? else {
                return Ok(None);
            };

            if self.buffer.starts_with(JobLog::COEUS.as_bytes()) {
                let line = &self.buffer[JobLog::COEUS.len()..end];
                let parsed = serde_json::from_slice::<Stamped<JobEvent>>(line);
                self.consume(end + 1);

                match parsed {
                    Ok(stamped) => return Ok(Some(LogItem::Event(stamped))),
                    Err(error) => debug!("skipping an unreadable job event: {error}"),
                }
                continue;
            }

            let mut taken = end + 1;
            while taken < max_nix {
                let Some(next) = self.line_end(taken).await? else {
                    break;
                };
                if self.buffer[taken..].starts_with(JobLog::COEUS.as_bytes()) || next + 1 > max_nix {
                    break;
                }
                taken = next + 1;
            }

            let nix = self.buffer[..taken].to_vec();
            self.consume(taken);
            return Ok(Some(LogItem::Nix(nix)));
        }
    }

    /// the next event, skipping nix lines without copying them
    pub async fn next_event(&mut self) -> Result<Option<Stamped<JobEvent>>> {
        while let Some(end) = self.line_end(0).await? {
            let event = self.buffer.starts_with(JobLog::COEUS.as_bytes()).then(|| {
                serde_json::from_slice::<Stamped<JobEvent>>(&self.buffer[JobLog::COEUS.len()..end])
            });
            self.consume(end + 1);

            match event {
                Some(Ok(stamped)) => return Ok(Some(stamped)),
                Some(Err(error)) => debug!("skipping an unreadable job event: {error}"),
                None => {}
            }
        }

        Ok(None)
    }

    /// the index of the newline ending the line that starts at `start`,
    /// reading more of the file when the buffer holds no complete line
    async fn line_end(&mut self, start: usize) -> Result<Option<usize>> {
        loop {
            if let Some(at) = memchr::memchr(b'\n', &self.buffer[start..]) {
                return Ok(Some(start + at));
            }

            if !self.fill().await? {
                return Ok(None);
            }
        }
    }

    /// reads more of the file into the buffer, false when nothing new is there
    async fn fill(&mut self) -> Result<bool> {
        let len = self.buffer.len();
        self.buffer.resize(len + Self::READ, 0);
        let read = self.file.read(&mut self.buffer[len..]).await?;
        self.buffer.truncate(len + read);
        Ok(read > 0)
    }

    fn consume(&mut self, count: usize) {
        self.buffer.drain(..count);
        self.offset += count as u64;
    }
}
