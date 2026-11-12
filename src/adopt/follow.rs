//! the job's progress on stdout, for `coeus bootstrap` reading it over ssh:
//!
//! ```text
//! @job <JobSummary json>        once, first
//! @coeus {"at":…,"event":…}     the job log's own lines, as they are
//! @nix <internal-json>
//! @at <offset>                  after each, where a follower resumes
//! ```
//!
//! a closed stdout (the ssh connection gone) just ends it; the runner does
//! not care.

use std::{io::ErrorKind, time::Duration};

use eyre::Result;
use tokio::{
    io::{AsyncWrite, AsyncWriteExt},
    time::{Instant, sleep},
};

use crate::{
    common::model::JobEvent,
    job::{JobDir, JobLog, LogItem},
};

/// how a stream ended
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ended {
    /// the log holds the job's finishing event
    Finished,
    /// no runner held the unfinished job for a while: running adopt again
    /// takes it up
    Unattended,
    /// nobody reads the stream any more
    Closed,
}

/// how long an unfinished job may go without a runner, such as while one
/// starts, before the stream gives up on it
const PATIENCE: Duration = Duration::from_secs(30);
const POLL: Duration = Duration::from_millis(200);
const NIX_BATCH: usize = 64 * 1024;

/// writes the job as it stands, then its log from `from` until the log
/// holds the finishing event
pub async fn stream<W: AsyncWrite + Unpin>(dir: &JobDir, from: u64, out: &mut W) -> Result<Ended> {
    stream_within(dir, from, out, PATIENCE).await
}

pub async fn stream_within<W: AsyncWrite + Unpin>(
    dir: &JobDir,
    from: u64,
    out: &mut W,
    patience: Duration,
) -> Result<Ended> {
    let log = dir.log();
    log.touch()?;

    let summary = dir.view().await?.summary;
    if !write(out, format!("@job {}\n", serde_json::to_string(&summary)?)).await? {
        return Ok(Ended::Closed);
    }

    let mut reader = log.reader(from).await?;
    let mut unattended: Option<Instant> = None;

    loop {
        if let Some(item) = reader.next(NIX_BATCH).await? {
            unattended = None;
            match relay(out, &item, reader.offset()).await? {
                Some(ended) => return Ok(ended),
                None => continue,
            }
        }

        // the finishing event is written before the summary, so with a
        // summary there the rest of the log is complete
        if dir.summary()?.is_some() {
            while let Some(item) = reader.next(NIX_BATCH).await? {
                if let Some(ended) = relay(out, &item, reader.offset()).await? {
                    return Ok(ended);
                }
            }
            return Ok(Ended::Finished);
        }

        match log.is_held()? {
            true => unattended = None,
            false if unattended.get_or_insert_with(Instant::now).elapsed() > patience => {
                return Ok(Ended::Unattended);
            }
            false => {}
        }
        sleep(POLL).await;
    }
}

/// writes one log item and where the next starts; `Some` when the stream
/// ends with it
async fn relay<W: AsyncWrite + Unpin>(
    out: &mut W,
    item: &LogItem,
    offset: u64,
) -> Result<Option<Ended>> {
    let (text, finished) = match item {
        LogItem::Nix(lines) => (String::from_utf8_lossy(lines).into_owned(), false),
        LogItem::Event(stamped) => (
            format!("{}{}\n", JobLog::COEUS, serde_json::to_string(stamped)?),
            matches!(stamped.event, JobEvent::Finished(_)),
        ),
    };

    if !write(out, format!("{text}@at {offset}\n")).await? {
        return Ok(Some(Ended::Closed));
    }
    Ok(finished.then_some(Ended::Finished))
}

/// false once nobody reads
async fn write<W: AsyncWrite + Unpin>(out: &mut W, text: String) -> Result<bool> {
    let written = async {
        out.write_all(text.as_bytes()).await?;
        out.flush().await
    };

    match written.await {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == ErrorKind::BrokenPipe => Ok(false),
        Err(error) => Err(error.into()),
    }
}
