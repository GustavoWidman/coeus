use eyre::{Result, eyre};

use crate::common::{
    model::{AttachFrame, JobId, JobSummary, MachineView, Outcome},
    proto::{AttachResize, Packet, TerminalSize},
    stream::EncryptedStream,
};

/// a machine's views as its daemon pushes them
pub struct Views {
    stream: EncryptedStream,
}

impl Views {
    pub(super) const fn new(stream: EncryptedStream) -> Self {
        Self { stream }
    }

    /// the next view, or `None` once the daemon hangs up
    pub async fn next(&mut self) -> Result<Option<MachineView>> {
        match self.stream.recv().await? {
            Some(Packet::MachineUpdate(update)) => Ok(Some(update.view)),
            Some(Packet::Error(error)) => Err(eyre!("{}", error.message)),
            Some(other) => Err(eyre!("unexpected packet {:#06x} in a watch", other.kind())),
            None => Ok(None),
        }
    }
}

/// a job's log as an attach streams it
pub struct Attachment {
    stream: EncryptedStream,
}

#[derive(Debug)]
pub enum AttachmentEvent {
    /// which job the attach resolved to, before any frame
    Started(JobSummary),
    Frames {
        frames: Vec<AttachFrame>,
        /// where a reconnect resumes
        end: u64,
    },
    /// terminal output of the machine's nom, for a drawn attach
    Screen(Vec<u8>),
    Ended {
        job: JobId,
        outcome: Outcome,
    },
}

impl Attachment {
    pub(super) const fn new(stream: EncryptedStream) -> Self {
        Self { stream }
    }

    /// the next piece of the log, or `None` if the daemon hung up early
    pub async fn next(&mut self) -> Result<Option<AttachmentEvent>> {
        Ok(Some(match self.stream.recv().await? {
            Some(Packet::AttachStarted(started)) => AttachmentEvent::Started(started.job),
            Some(Packet::AttachBatch(batch)) => AttachmentEvent::Frames {
                frames: batch.frames,
                end: batch.end,
            },
            Some(Packet::AttachScreen(screen)) => AttachmentEvent::Screen(screen.bytes),
            Some(Packet::AttachEnded(ended)) => AttachmentEvent::Ended {
                job: ended.job,
                outcome: ended.outcome,
            },
            Some(Packet::Error(error)) => return Err(eyre!("{}", error.message)),
            Some(other) => {
                return Err(eyre!(
                    "unexpected packet {:#06x} in an attach",
                    other.kind()
                ));
            }
            None => return Ok(None),
        }))
    }

    /// redraws a drawn attach at the terminal's new size
    pub async fn resize(&mut self, size: TerminalSize) -> Result<()> {
        self.stream.send(AttachResize { size }).await
    }
}
