use std::{
    io::{ErrorKind, Write},
    process::Stdio,
    time::Duration,
};

use eyre::Result;
use tokio::{
    io::AsyncWriteExt,
    process::{Child, ChildStdin, Command},
    time::{sleep, timeout},
};

use crate::{
    client::{AttachmentEvent, Fleet},
    common::model::{
        AttachFrame, ClosureDiff, Hostname, JobEvent, JobSelector, JobSummary, Outcome, Timestamp,
    },
    ui::{journal, say},
};

/// how an attach shows the job's log
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// nix-output-monitor's live build view, coeus events in between
    Nom,
    /// nix's internal-json lines on stdout, for piping
    Raw,
    /// readable lines: events and nix messages
    Transcript,
    /// nothing shown, only the outcome and the diff collected
    Silent,
}

/// a job's log streamed from one machine, resumed where it left off when
/// the connection drops
pub struct Attach<'a> {
    pub fleet: &'a Fleet,
    pub host: &'a Hostname,
    pub job: JobSelector,
    pub mode: Mode,
    /// stop once nothing new arrived for this long, instead of following
    /// the job to its end
    pub idle: Option<Duration>,
    /// how many package changes the ending lists
    pub diff_limit: Option<usize>,
}

/// what an attach saw of its job
pub struct Attached {
    pub job: JobSummary,
    /// `None` when it stopped before the job finished
    pub outcome: Option<Outcome>,
    pub diff: Option<ClosureDiff>,
}

impl Attach<'_> {
    const BACKOFF: [Duration; 4] = [
        Duration::from_secs(1),
        Duration::from_secs(2),
        Duration::from_secs(5),
        Duration::from_secs(10),
    ];

    pub async fn run(mut self) -> Result<Option<Attached>> {
        let mut from = 0;
        let mut sink: Option<Sink> = None;
        let mut seen: Option<Attached> = None;
        let mut failures = 0usize;
        let interrupt = tokio::signal::ctrl_c();
        tokio::pin!(interrupt);

        loop {
            let attachment = match self.fleet.connect(self.host).await {
                Ok(client) => client.attach(self.job, from, None).await,
                Err(error) => Err(error),
            };
            let mut attachment = match attachment {
                Ok(attachment) => attachment,
                Err(error) if sink.is_none() => return Err(error),
                Err(_) => {
                    sleep(Self::BACKOFF[failures.min(Self::BACKOFF.len() - 1)]).await;
                    failures += 1;
                    continue;
                }
            };

            loop {
                let next = async {
                    match self.idle {
                        Some(idle) => timeout(idle, attachment.next()).await.unwrap_or(Ok(None)),
                        None => attachment.next().await,
                    }
                };

                let event = tokio::select! {
                    _ = &mut interrupt => {
                        if let Some(sink) = sink.take() {
                            sink.close().await?;
                        }
                        if let Some(seen) = &seen {
                            eprintln!(
                                "{}",
                                say::aside(format!("detached from {} job {}, it keeps running", self.host, seen.job.id))
                            );
                        }
                        return Ok(seen);
                    }
                    event = next => event,
                };

                match event {
                    Ok(Some(AttachmentEvent::Started(job))) => {
                        if sink.is_none() {
                            self.header(&job);
                            sink = Some(Sink::open(self.mode).await?);
                        }
                        self.job = JobSelector::Id(job.id);
                        seen.get_or_insert(Attached {
                            job,
                            outcome: None,
                            diff: None,
                        });
                    }
                    Ok(Some(AttachmentEvent::Frames { frames, end })) => {
                        failures = 0;
                        from = end;
                        for frame in &frames {
                            if let (AttachFrame::Event(stamped), Some(seen)) =
                                (frame, seen.as_mut())
                            {
                                seen.track(&stamped.event, stamped.at);
                            }
                            if let Some(sink) = sink.as_mut() {
                                sink.frame(frame).await?;
                            }
                        }
                    }
                    Ok(Some(AttachmentEvent::Screen(_))) => {}
                    Ok(Some(AttachmentEvent::Ended { outcome, .. })) => {
                        if let Some(sink) = sink.take() {
                            sink.close().await?;
                        }
                        let Some(mut seen) = seen else {
                            return Ok(None);
                        };
                        seen.outcome = Some(outcome);
                        self.ending(&seen);
                        return Ok(Some(seen));
                    }
                    Ok(None) if self.idle.is_some() => {
                        if let Some(sink) = sink.take() {
                            sink.close().await?;
                        }
                        return Ok(seen);
                    }
                    Ok(None) | Err(_) => {
                        if let Some(sink) = sink.as_mut() {
                            sink.line(&say::warn(format!(
                                "lost the connection to {}, reconnecting",
                                self.host
                            )))
                            .await?;
                        }
                        break;
                    }
                }
            }

            sleep(Self::BACKOFF[failures.min(Self::BACKOFF.len() - 1)]).await;
            failures += 1;
        }
    }

    fn header(&self, job: &JobSummary) {
        if self.mode == Mode::Silent {
            return;
        }
        let lines = journal::header(self.host.as_str(), job, Timestamp::now());
        self.print(&lines);
        self.print(&[String::new()]);
    }

    fn ending(&self, seen: &Attached) {
        let Some(outcome) = &seen.outcome else {
            return;
        };
        if self.mode == Mode::Silent {
            return;
        }

        let lines = journal::ending(
            self.host.as_str(),
            &seen.job,
            outcome,
            seen.diff.as_ref(),
            self.diff_limit,
            Timestamp::now(),
        );
        self.print(&[String::new()]);
        self.print(&lines);
    }

    /// to stdout, unless stdout carries raw nix lines
    fn print(&self, lines: &[String]) {
        for line in lines {
            match self.mode {
                Mode::Raw => eprintln!("{line}"),
                _ => println!("{line}"),
            }
        }
    }
}

impl Attached {
    fn track(&mut self, event: &JobEvent, at: Timestamp) {
        match event {
            JobEvent::Diff(diff) => self.diff = Some(diff.clone()),
            JobEvent::Finished(_) => self.job.finished_at = Some(at),
            JobEvent::Started { .. } => {
                self.job.started_at.get_or_insert(at);
            }
            _ => {}
        }
    }
}

/// where an attach writes the log
enum Sink {
    Nom { child: Child, stdin: ChildStdin },
    Raw,
    Transcript,
    Silent,
}

impl Sink {
    async fn open(mode: Mode) -> Result<Self> {
        Ok(match mode {
            Mode::Nom => match Self::nom() {
                Ok(sink) => sink,
                Err(error) if error.kind() == ErrorKind::NotFound => {
                    println!(
                        "{}",
                        say::warn("nom is not installed, showing the plain transcript instead")
                    );
                    Self::Transcript
                }
                Err(error) => return Err(error.into()),
            },
            Mode::Raw => Self::Raw,
            Mode::Transcript => Self::Transcript,
            Mode::Silent => Self::Silent,
        })
    }

    fn nom() -> std::io::Result<Self> {
        let mut child = Command::new("nom")
            .arg("--json")
            .stdin(Stdio::piped())
            .kill_on_drop(true)
            .spawn()?;
        let stdin = child.stdin.take().expect("nom's stdin is piped");
        Ok(Self::Nom { child, stdin })
    }

    async fn frame(&mut self, frame: &AttachFrame) -> Result<()> {
        match (self, frame) {
            (Self::Nom { stdin, .. }, AttachFrame::Nix(bytes)) => stdin.write_all(bytes).await?,
            (Self::Nom { stdin, .. }, AttachFrame::Event(stamped)) => {
                if let Some(line) = journal::event(stamped) {
                    stdin.write_all(format!("{line}\n").as_bytes()).await?;
                }
            }
            (Self::Raw, AttachFrame::Nix(bytes)) => std::io::stdout().lock().write_all(bytes)?,
            (Self::Raw, AttachFrame::Event(stamped)) => {
                if let Some(line) = journal::event(stamped) {
                    eprintln!("{line}");
                }
            }
            (Self::Transcript, frame) => {
                for line in journal::frame(frame) {
                    println!("{line}");
                }
            }
            (Self::Silent, _) => {}
        }
        Ok(())
    }

    async fn line(&mut self, text: &str) -> Result<()> {
        match self {
            Self::Nom { stdin, .. } => stdin.write_all(format!("{text}\n").as_bytes()).await?,
            Self::Raw => eprintln!("{text}"),
            Self::Transcript => println!("{text}"),
            Self::Silent => {}
        }
        Ok(())
    }

    /// lets nom draw its last frame and exit
    async fn close(self) -> Result<()> {
        match self {
            Self::Nom { mut child, stdin } => {
                drop(stdin);
                child.wait().await?;
            }
            Self::Raw => std::io::stdout().flush()?,
            Self::Transcript | Self::Silent => {}
        }
        Ok(())
    }
}
