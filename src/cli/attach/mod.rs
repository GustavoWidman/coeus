mod sink;
mod terminal;

use std::time::Duration;

use eyre::Result;
use tokio::{
    signal::unix::{Signal, SignalKind, signal},
    time::{sleep, timeout},
};

use self::sink::Sink;
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
    /// on a terminal, nix-output-monitor's build view with coeus events in
    /// between, drawn on the machine at this terminal's size; elsewhere, or
    /// when the machine cannot draw, the transcript
    Live,
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
        let mut resizes = match self.mode {
            Mode::Live => Some(signal(SignalKind::window_change())?),
            _ => None,
        };

        loop {
            let terminal = match self.mode {
                Mode::Live => terminal::size(),
                _ => None,
            };
            let attachment = match self.fleet.connect(self.host).await {
                Ok(client) => client.attach(self.job, from, terminal).await,
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
                            sink.close()?;
                        }
                        if let Some(seen) = &seen {
                            eprintln!(
                                "{}",
                                say::aside(format!("detached from {} job {}, it keeps running", self.host, seen.job.id))
                            );
                        }
                        return Ok(seen);
                    }
                    _ = resized(&mut resizes) => None,
                    event = next => Some(event),
                };

                let Some(event) = event else {
                    if let (Some(size), true) =
                        (terminal::size(), sink.as_ref().is_some_and(Sink::draws))
                    {
                        // a lost connection shows on the next read
                        let _ = attachment.resize(size).await;
                    }
                    continue;
                };

                match event {
                    Ok(Some(AttachmentEvent::Started(job))) => {
                        if sink.is_none() {
                            self.header(&job);
                            sink = Some(Sink::open(self.mode, terminal.is_some()));
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
                        if let Some(seen) = seen.as_mut() {
                            for frame in &frames {
                                if let AttachFrame::Event(stamped) = frame {
                                    seen.track(&stamped.event, stamped.at);
                                }
                            }
                        }
                        if let Some(sink) = sink.as_mut() {
                            sink.frames(&frames)?;
                        }
                    }
                    Ok(Some(AttachmentEvent::Screen(bytes))) => {
                        failures = 0;
                        if let Some(sink) = sink.as_mut() {
                            sink.screen(&bytes)?;
                        }
                    }
                    Ok(Some(AttachmentEvent::Ended { outcome, .. })) => {
                        if let Some(sink) = sink.take() {
                            sink.close()?;
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
                            sink.close()?;
                        }
                        return Ok(seen);
                    }
                    Ok(None) | Err(_) => {
                        if let Some(sink) = sink.as_mut() {
                            sink.line(&say::warn(format!(
                                "lost the connection to {}, reconnecting",
                                self.host
                            )))?;
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

/// the terminal's next resize, never without a terminal to follow
async fn resized(resizes: &mut Option<Signal>) {
    match resizes {
        Some(resizes) => {
            resizes.recv().await;
        }
        None => std::future::pending().await,
    }
}
