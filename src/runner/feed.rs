use std::time::{Duration, Instant};

use eyre::Result;

use crate::{
    common::model::JobEvent,
    job::LogWriter,
    nix::{Line, LogFilter, LogFold},
};

/// one process's output on its way into the job log: folded into progress
/// and the tail that explains a failure, filtered, and appended
pub struct Feed<'a> {
    writer: &'a mut LogWriter,
    fold: LogFold,
    filter: LogFilter,
    unflushed: bool,
    reported: Instant,
}

impl<'a> Feed<'a> {
    /// how often progress is recorded at most
    const PROGRESS: Duration = Duration::from_millis(250);

    pub fn new(writer: &'a mut LogWriter) -> Self {
        Self {
            writer,
            fold: LogFold::new(),
            filter: LogFilter::new(),
            unflushed: false,
            reported: Instant::now(),
        }
    }

    pub fn line(&mut self, raw: &str) -> Result<()> {
        let line = Line::parse(raw);
        self.fold.apply(&line);

        if self.filter.keep(&line) {
            self.writer.output(raw)?;
            self.unflushed = true;
        }
        Ok(())
    }

    /// makes what was written visible and records progress, at most every
    /// [Feed::PROGRESS]
    pub fn tick(&mut self) -> Result<()> {
        if self.reported.elapsed() >= Self::PROGRESS {
            self.report()?;
        }
        if std::mem::take(&mut self.unflushed) {
            self.writer.flush()?;
        }
        Ok(())
    }

    /// flushes and records the last progress, once the process is done
    pub fn settle(&mut self) -> Result<()> {
        self.report()?;
        self.unflushed = false;
        self.writer.flush()
    }

    pub const fn fold(&self) -> &LogFold {
        &self.fold
    }

    fn report(&mut self) -> Result<()> {
        self.reported = Instant::now();
        if let Some(progress) = self.fold.snapshot() {
            self.writer.event(JobEvent::Progress(progress))?;
        }
        Ok(())
    }
}
