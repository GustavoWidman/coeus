//! a job's log as a browser reads it: typed lines, each message's id the
//! machine's own log offset, which a browser sends back to resume, on this
//! host or any other

use axum::body::{Body, Bytes};
use eyre::{Result, eyre};
use tokio::{
    sync::mpsc,
    time::{sleep, timeout},
};

use super::{
    source::{Feed, Source},
    stream::{HEARTBEAT, Kind, UNWRITABLE, message},
    transcript::Transcript,
    view::{JobCard, LogEvent},
};
use crate::{client::AttachmentEvent, common::model::Hostname};

/// job `job`'s log on `host` from offset `from`, live until the job ends;
/// an error if the machine cannot be reached or knows no such job
pub async fn follow(source: &Source, host: &Hostname, job: u64, from: u64) -> Result<Body> {
    let mut feed = source.attach(host, job, from).await?;
    let first = feed
        .next()
        .await?
        .ok_or_else(|| eyre!("{host} hung up before sending job {job}'s log"))?;

    // the machine's connection is read on its own, so waiting on it never
    // cuts a read short to send a heartbeat
    let (events, mut received) = mpsc::channel(1);
    tokio::spawn(read(feed, first, events));

    let (frames, receiver) = mpsc::channel::<Bytes>(1);
    tokio::spawn(async move {
        let mut transcript = Transcript::default();
        let mut offset = from;
        loop {
            let frame = tokio::select! {
                event = received.recv() => match event {
                    Some(event) => match translate(event, &mut transcript, &mut offset) {
                        Some(event) => message(offset, &event),
                        None => continue,
                    },
                    None => return,
                },
                () = sleep(HEARTBEAT) => ":\n\n".to_owned(),
            };
            let sent = timeout(UNWRITABLE, frames.send(Bytes::from(frame))).await;
            if !matches!(sent, Ok(Ok(()))) {
                return;
            }
        }
    });

    Ok(Body::from_stream(futures_util::stream::unfold(
        receiver,
        |mut receiver| async move {
            let frame = receiver.recv().await?;
            Some((Ok::<_, std::convert::Infallible>(frame), receiver))
        },
    )))
}

async fn read(mut feed: Feed, first: AttachmentEvent, events: mpsc::Sender<AttachmentEvent>) {
    let mut event = first;
    loop {
        let ended = matches!(event, AttachmentEvent::Ended { .. });
        if events.send(event).await.is_err() || ended {
            return;
        }
        event = match feed.next().await {
            Ok(Some(event)) => event,
            _ => return,
        };
    }
}

/// what a browser is told of `event`, if anything
fn translate(
    event: AttachmentEvent,
    transcript: &mut Transcript,
    offset: &mut u64,
) -> Option<LogEvent> {
    match event {
        AttachmentEvent::Started(job) => Some(LogEvent::Started {
            job: Box::new(JobCard::from(&job)),
        }),
        AttachmentEvent::Frames { frames, end } => {
            *offset = end;
            let lines = transcript.lines(&frames);
            (!lines.is_empty()).then_some(LogEvent::Lines { lines, end })
        }
        AttachmentEvent::Ended { outcome, .. } => Some(LogEvent::Ended {
            outcome: (&outcome).into(),
        }),
        AttachmentEvent::Screen(_) => None,
    }
}

impl Kind for LogEvent {
    fn kind(&self) -> &'static str {
        match self {
            Self::Started { .. } => "started",
            Self::Lines { .. } => "lines",
            Self::Ended { .. } => "ended",
        }
    }
}
