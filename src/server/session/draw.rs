use eyre::Result;
use log::debug;
use tokio::{process::ChildStdin, sync::mpsc};

use crate::{
    common::{
        model::{AttachFrame, JobEvent, JobId, Outcome, Stamped},
        proto::{AttachBatch, AttachEnded, AttachScreen, Error, Packet, TerminalSize},
        stream::{EncryptedReadHalf, EncryptedWriteHalf},
    },
    job::JobLog,
    server::nom::{Feed, Nom},
};

/// an event nom has read, and the log offset after it
type Fed = (Stamped<JobEvent>, u64);

/// the most events one batch carries
const EVENTS: usize = 64;

/// streams what `nom` draws of the job's log, and the job's events in
/// batches, until nom has drawn the job's end. the client's resizes reach
/// nom's terminal; its hangup ends nom
pub(super) async fn draw(
    reader: &mut EncryptedReadHalf,
    writer: &mut EncryptedWriteHalf,
    job: JobId,
    log: JobLog,
    mut nom: Nom,
    input: ChildStdin,
) -> Result<()> {
    let (fed, mut events) = mpsc::unbounded_channel();
    let feed = Feed::new(log, fed).run(input);
    tokio::pin!(feed);
    let mut feeding = true;
    let mut outcome = None;

    loop {
        tokio::select! {
            result = &mut feed, if feeding => {
                feeding = false;
                if let Err(error) = result {
                    debug!("stopped feeding nom: {error}");
                }
            }
            Some(first) = events.recv() => {
                send_events(writer, batch(&mut events, first), &mut outcome).await?;
            }
            output = nom.output() => match output {
                Some(bytes) => writer.send(AttachScreen { bytes }).await?,
                None => break,
            },
            packet = reader.recv() => match packet {
                Ok(Some(Packet::AttachResize(resize))) => {
                    let TerminalSize { columns, rows } = resize.size;
                    debug!("redrawing job {job} at {columns}x{rows}");
                    nom.resize(resize.size)?;
                }
                _ => return Ok(()),
            },
        }
    }

    while let Ok(first) = events.try_recv() {
        send_events(writer, batch(&mut events, first), &mut outcome).await?;
    }
    nom.wait().await?;

    match outcome {
        Some(outcome) => writer.send(AttachEnded { job, outcome }).await,
        None => {
            let message = "nom stopped before the job finished".to_owned();
            writer.send(Error { message }).await
        }
    }
}

/// `first` and the events queued behind it
fn batch(events: &mut mpsc::UnboundedReceiver<Fed>, first: Fed) -> Vec<Fed> {
    let mut batch = vec![first];
    while batch.len() < EVENTS
        && let Ok(next) = events.try_recv()
    {
        batch.push(next);
    }
    batch
}

async fn send_events(
    writer: &mut EncryptedWriteHalf,
    events: Vec<Fed>,
    outcome: &mut Option<Outcome>,
) -> Result<()> {
    let end = events.last().map_or(0, |(_, end)| *end);
    let frames = events
        .into_iter()
        .map(|(stamped, _)| {
            if let JobEvent::Finished(finished) = &stamped.event {
                *outcome = Some(finished.clone());
            }
            AttachFrame::Event(stamped)
        })
        .collect();
    writer.send(AttachBatch { frames, end }).await
}
