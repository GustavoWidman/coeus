use std::{
    fs::File,
    io::{BufWriter, Write},
};

use eyre::Result;

use super::JobLog;
use crate::common::model::{JobEvent, Outcome, Stamped};

/// append access to a job log. owning one means holding the log's lock,
/// so there is never more than one writer.
pub struct LogWriter {
    out: BufWriter<File>,
}

impl LogWriter {
    pub(super) fn new(file: File) -> Self {
        Self {
            out: BufWriter::with_capacity(64 * 1024, file),
        }
    }

    /// one line of process output. nix's own lines are kept verbatim; any
    /// other text is wrapped as a nix message, so `nom` shows it in order and
    /// no process can print a line that reads as a coeus event.
    pub fn output(&mut self, line: &str) -> Result<()> {
        let line = line.trim_end_matches(['\n', '\r']);

        if line.starts_with(JobLog::NIX) && line.len() <= JobLog::MAX_LINE {
            self.out.write_all(line.as_bytes())?;
            self.out.write_all(b"\n")?;
            return Ok(());
        }

        let text = line.strip_prefix(JobLog::NIX).unwrap_or(line);
        let text = match text.len() > JobLog::MAX_LINE / 2 {
            true => &text[..text.floor_char_boundary(JobLog::MAX_LINE / 2)],
            false => text,
        };

        self.out.write_all(JobLog::NIX.as_bytes())?;
        self.out.write_all(br#"{"action":"msg","level":3,"msg":"#)?;
        serde_json::to_writer(&mut self.out, text)?;
        self.out.write_all(b"}\n")?;
        Ok(())
    }

    pub fn event(&mut self, event: JobEvent) -> Result<Stamped<JobEvent>> {
        let stamped = Stamped::now(event);

        self.out.write_all(JobLog::COEUS.as_bytes())?;
        serde_json::to_writer(&mut self.out, &stamped)?;
        self.out.write_all(b"\n")?;
        self.out.flush()?;

        Ok(stamped)
    }

    /// makes buffered output visible to readers
    pub fn flush(&mut self) -> Result<()> {
        Ok(self.out.flush()?)
    }

    /// writes the terminal event and syncs the log, then releases the lock.
    /// nothing can follow it.
    pub fn finish(mut self, outcome: Outcome) -> Result<Stamped<JobEvent>> {
        let stamped = self.event(JobEvent::Finished(outcome))?;
        self.out.get_ref().sync_all()?;
        Ok(stamped)
    }
}
