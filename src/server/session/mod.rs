mod draw;

use std::time::Duration;

use eyre::{Result, eyre};
use log::{debug, warn};
use tokio::time::sleep;

use super::{fit::Fit, nom::Nom};
use crate::{
    common::{
        model::{AttachFrame, JobEvent, JobSelector, NarHash, Order},
        proto::{
            Ack, AttachBatch, AttachEnded, AttachStarted, CancelResponse, DeployAccepted,
            DeployRejected, Error, GenerationsResponse, HistoryResponse, MachineUpdate, Packet,
            ResolveResponse, TerminalSize, UploadDone, UploadReady,
        },
        stream::EncryptedStream,
    },
    engine::{EngineHandle, Host},
    job::LogItem,
};

/// one client connection: a sequence of conversations, the last of which
/// may be a watch or an attach that streams until either side hangs up
pub(super) struct Session<H: Host> {
    stream: EncryptedStream,
    engine: EngineHandle<H>,
}

impl<H: Host> Session<H> {
    /// the most a batch collects before sending, leaving room for one
    /// maximal log line within a single frame
    const BATCH: usize = 24 * 1024;
    const WATCH_INTERVAL: Duration = Duration::from_millis(100);
    const CAUGHT_UP: Duration = Duration::from_millis(150);

    pub const fn new(stream: EncryptedStream, engine: EngineHandle<H>) -> Self {
        Self { stream, engine }
    }

    pub async fn run(mut self) -> Result<()> {
        while let Some(packet) = self.stream.recv().await? {
            debug!("{} asks for {:#06x}", self.stream.peer(), packet.kind());

            match packet {
                Packet::DeployRequest(request) => self.deploy(request.order).await?,
                Packet::CancelRequest(request) => {
                    let report = self.engine.cancel(request.job).await?;
                    self.stream.send(CancelResponse { report }).await?;
                }
                Packet::ResolveRequest(request) => {
                    match self.engine.resolve(&request.git_ref).await {
                        Ok(pinned) => self.stream.send(ResolveResponse { pinned }).await?,
                        Err(error) => self.refuse(error.to_string()).await?,
                    }
                }
                Packet::HistoryRequest(request) => {
                    let jobs = self.engine.history(request.limit as usize).await?;
                    let jobs = jobs.into_iter().map(Fit::fit).collect();
                    self.stream.send(HistoryResponse { jobs }).await?;
                }
                Packet::GenerationsRequest(_) => match self.engine.generations().await {
                    Ok(generations) => {
                        self.stream
                            .send(GenerationsResponse { generations })
                            .await?
                    }
                    Err(error) => self.refuse(error.to_string()).await?,
                },
                Packet::RebootRequest(_) => self.reboot().await?,
                Packet::WatchRequest(request) if request.follow => return self.watch().await,
                Packet::WatchRequest(_) => {
                    let view = self.engine.watch().borrow().clone().fit();
                    self.stream.send(MachineUpdate { view }).await?;
                }
                Packet::AttachRequest(request) => {
                    return self
                        .attach(request.job, request.from, request.terminal)
                        .await;
                }
                Packet::UploadBegin(begin) => self.upload(begin.nar_hash, begin.size).await?,
                other => {
                    self.refuse(format!("unexpected packet {:#06x}", other.kind()))
                        .await?
                }
            }
        }

        Ok(())
    }

    async fn deploy(&mut self, order: Order) -> Result<()> {
        match self.engine.deploy(order).await? {
            Ok(admission) => self.stream.send(DeployAccepted { admission }).await,
            Err(rejection) => self.stream.send(DeployRejected { rejection }).await,
        }
    }

    async fn reboot(&mut self) -> Result<()> {
        let activating = self
            .engine
            .watch()
            .borrow()
            .running
            .as_ref()
            .and_then(|running| running.summary.state.phase())
            .is_some_and(|phase| !phase.is_cancellable());

        if activating {
            return self
                .refuse("a job is activating, reboot after it finishes".into())
                .await;
        }

        self.stream.send(Ack {}).await?;
        self.engine.reboot().await
    }

    /// takes a working tree for a later `Source::Snapshot`, skipping the
    /// transfer when the daemon already has it
    async fn upload(&mut self, hash: NarHash, size: u64) -> Result<()> {
        let snapshots = self.engine.snapshots().clone();

        if snapshots.has(&hash) {
            self.stream.send(UploadReady { offset: size }).await?;
            return self.stream.send(UploadDone {}).await;
        }

        let mut receiving = match snapshots.receive(hash, size).await {
            Ok(receiving) => receiving,
            Err(error) => return self.refuse(error.to_string()).await,
        };
        self.stream.send(UploadReady { offset: 0 }).await?;

        let mut complete = size == 0;
        while !complete {
            match self.stream.recv().await? {
                Some(Packet::UploadChunk(chunk)) => complete = receiving.write(&chunk.data).await?,
                _ => return Err(eyre!("the upload ended early")),
            }
        }

        match receiving.finish(&snapshots).await {
            Ok(()) => self.stream.send(UploadDone {}).await,
            Err(error) => self.refuse(error.to_string()).await,
        }
    }

    async fn refuse(&mut self, message: String) -> Result<()> {
        self.stream.send(Error { message }).await
    }

    /// pushes the machine's view whenever it changes, at most ten times a
    /// second; a slow client skips intermediate views instead of queuing them
    async fn watch(mut self) -> Result<()> {
        let mut views = self.engine.watch();
        let (reader, writer) = self.stream.split();

        loop {
            let view = views.borrow_and_update().clone().fit();
            writer.send(MachineUpdate { view }).await?;

            tokio::select! {
                _ = sleep(Self::WATCH_INTERVAL) => {}
                _ = reader.recv() => return Ok(()),
            }

            tokio::select! {
                changed = views.changed() => if changed.is_err() { return Ok(()) },
                _ = reader.recv() => return Ok(()),
            }
        }
    }

    /// streams a job's log until its terminal event: drawn by nom at the
    /// size of the client's `terminal` when it has one and nom runs here,
    /// else as frames from `from`
    async fn attach(
        mut self,
        selector: JobSelector,
        from: u64,
        terminal: Option<TerminalSize>,
    ) -> Result<()> {
        let Some((summary, dir)) = self.engine.locate(selector) else {
            return self.refuse("no such job".into()).await;
        };

        let job = summary.id;
        self.stream
            .send(AttachStarted { job: summary.fit() })
            .await?;

        let (reader, writer) = self.stream.split();
        let log = dir.log();

        while !log.exists() {
            tokio::select! {
                _ = sleep(Self::CAUGHT_UP) => {}
                _ = reader.recv() => return Ok(()),
            }
        }

        if let Some(size) = terminal {
            match Nom::spawn(size) {
                Ok((nom, input)) => {
                    debug!("drawing job {job} at {}x{}", size.columns, size.rows);
                    return draw::draw(reader, writer, job, log, nom, input).await;
                }
                Err(error) => warn!("cannot draw attaches with nom, sending logs instead: {error}"),
            }
        }

        let mut lines = log.reader(from).await?;
        loop {
            let mut frames = Vec::new();
            let mut size = 0;
            let mut outcome = None;

            while size < Self::BATCH && outcome.is_none() {
                match lines.next(Self::BATCH - size).await? {
                    Some(LogItem::Nix(bytes)) => {
                        size += bytes.len();
                        frames.push(AttachFrame::Nix(bytes));
                    }
                    Some(LogItem::Event(event)) => {
                        if let JobEvent::Finished(finished) = &event.event {
                            outcome = Some(finished.clone());
                        }
                        size += 256;
                        frames.push(AttachFrame::Event(event));
                    }
                    None => break,
                }
            }

            if !frames.is_empty() {
                let end = lines.offset();
                writer.send(AttachBatch { frames, end }).await?;
            }

            if let Some(outcome) = outcome {
                return writer.send(AttachEnded { job, outcome }).await;
            }

            if size == 0 {
                tokio::select! {
                    _ = sleep(Self::CAUGHT_UP) => {}
                    _ = reader.recv() => return Ok(()),
                }
            }
        }
    }
}
