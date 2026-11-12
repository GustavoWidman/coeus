use std::{borrow::Cow, time::Duration};

use eyre::Result;
use tokio::{io::AsyncWriteExt, process::ChildStdin, sync::mpsc, time::sleep};

use crate::{
    common::model::{JobEvent, Stamped},
    job::{JobLog, LogItem},
    ui::journal,
};

/// a job's log as nom reads it: nix's lines verbatim and coeus's events as
/// the lines `coeus attach` shows between them. each event also goes to
/// `fed` with the log offset after it, once nom has it
pub(in crate::server) struct Feed {
    log: JobLog,
    fed: mpsc::UnboundedSender<(Stamped<JobEvent>, u64)>,
}

impl Feed {
    const POLL: Duration = Duration::from_millis(150);

    pub const fn new(log: JobLog, fed: mpsc::UnboundedSender<(Stamped<JobEvent>, u64)>) -> Self {
        Self { log, fed }
    }

    /// writes the log from its start through the job's `Finished` event,
    /// following it while the job runs, then closes nom's input
    pub async fn run(self, mut nom: ChildStdin) -> Result<()> {
        let mut lines = self.log.reader(0).await?;
        loop {
            let Some(item) = lines.next(JobLog::MAX_LINE).await? else {
                sleep(Self::POLL).await;
                continue;
            };

            nom.write_all(&Self::bytes(&item)).await?;
            if let LogItem::Event(stamped) = item {
                let finished = matches!(stamped.event, JobEvent::Finished(_));
                let _ = self.fed.send((stamped, lines.offset()));
                if finished {
                    return Ok(());
                }
            }
        }
    }

    /// what nom reads for one record of the log
    pub fn bytes(item: &LogItem) -> Cow<'_, [u8]> {
        match item {
            LogItem::Nix(lines) => Cow::Borrowed(lines),
            LogItem::Event(stamped) => match journal::event(stamped) {
                Some(line) => Cow::Owned(format!("{line}\n").into_bytes()),
                None => Cow::Borrowed(&[]),
            },
        }
    }
}
