//! the terminal during a bootstrap: the checklist redrawn in place, and
//! everything that needs the person (a choice, a password, a host key)
//! asked inside it, so the live region and the questions never fight over
//! the cursor. ssh and sudo prompt on the terminal themselves: for those the
//! console draws a last frame, hands the terminal over, and starts a new
//! frame below when they are done.

#[cfg(test)]
mod test;

use std::{fmt, future::Future, io::Write, sync::Once, time::Duration};

use crossterm::{
    cursor,
    event::{Event, EventStream},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode},
};
use eyre::{Result, eyre};
use futures_util::StreamExt;
use tokio::{
    signal::unix::{SignalKind, signal},
    sync::{mpsc, oneshot},
    time::{MissedTickBehavior, interval},
};
use tokio_util::sync::CancellationToken;

use crate::{
    bootstrap::{Finale, Step, state::StateDir},
    common::model::Timestamp,
    ui::{
        Frame, Screen,
        ask::{Answer, Prompt},
        checklist::{self, Checklist, Mark, Row},
        finale, say,
    },
};

/// cloned into every step; all it does is send to the task owning the
/// terminal
#[derive(Clone)]
pub struct Console {
    requests: mpsc::UnboundedSender<Request>,
    /// cancelled by ctrl-c; steps check it between units of work
    pub cancelled: CancellationToken,
    /// a terminal to draw and ask on
    pub live: bool,
}

enum Request {
    Row(Row),
    Note(String),
    Ask(Prompt, oneshot::Sender<Answer>),
    /// draw a last frame, then answer and wait for the hand-back
    HandOver(String, oneshot::Sender<oneshot::Sender<()>>),
    /// the run ended: settle the rows, then print these
    Finish(Vec<String>, oneshot::Sender<()>),
    /// the checklist as it stands, for a test to read
    #[cfg(test)]
    Peek(oneshot::Sender<Checklist>),
}

/// a question nobody can answer: no terminal, and no `--answer` for it
#[derive(Debug)]
pub struct Unanswerable {
    /// the contract question's id, when it is one
    pub id: Option<String>,
    pub question: String,
}

impl fmt::Display for Unanswerable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.id {
            Some(id) => write!(
                f,
                "the repository asks `{id}` and there is no terminal: pass --answer {id}=<value>"
            ),
            None => write!(f, "{:?} needs a terminal to answer", self.question),
        }
    }
}

impl std::error::Error for Unanswerable {}

impl Console {
    /// starts the task that owns the terminal: a [Screen] when live,
    /// [checklist::plain] lines otherwise. raw mode is on only while a
    /// prompt is open, so ctrl-c stays a signal otherwise.
    pub fn start(live: bool, checklist: Checklist) -> Result<Self> {
        let (requests, received) = mpsc::unbounded_channel();
        let cancelled = CancellationToken::new();
        let interrupts = signal(SignalKind::interrupt())?;

        if live {
            guard()?;
        }
        let out = match live {
            true => Out::Live(Some(Screen::new()?)),
            false => Out::Plain {
                shown: Box::new(Checklist::new(String::new(), Vec::new())),
            },
        };
        if let Out::Plain { .. } = out {
            println!("{}", checklist.title);
            for line in &checklist.subtitle {
                println!("{line}");
            }
        }

        let terminal = Terminal {
            checklist,
            out,
            asking: None,
            cancelled: cancelled.clone(),
        };
        tokio::spawn(terminal.serve(received, interrupts));

        Ok(Self {
            requests,
            cancelled,
            live,
        })
    }

    pub fn update(&self, row: Row) {
        let _ = self.requests.send(Request::Row(row));
    }

    /// shorthand for a row with no live lines
    pub fn mark(&self, step: Step, mark: Mark, detail: impl Into<String>) {
        self.update(Row::new(step, mark, detail));
    }

    pub fn note(&self, line: impl Into<String>) {
        let _ = self.requests.send(Request::Note(line.into()));
    }

    /// the person's answer, or [Answer::Cancelled] after ctrl-c. off a
    /// terminal this fails with [Unanswerable], which names the question.
    pub async fn ask(&self, prompt: Prompt) -> Result<Answer> {
        if !self.live {
            return Err(Unanswerable {
                id: prompt.id,
                question: prompt.question,
            }
            .into());
        }

        let (reply, answer) = oneshot::channel();
        self.requests
            .send(Request::Ask(prompt, reply))
            .map_err(|_| stopped())?;
        answer.await.map_err(|_| stopped())
    }

    /// gives the terminal to `work` (ssh asking a login password, sudo
    /// asking for the admin's key), saying `why` first
    pub async fn hand_over<T>(&self, why: &str, work: impl Future<Output = T>) -> T {
        let (ready, handed) = oneshot::channel();
        let back = match self.requests.send(Request::HandOver(why.to_owned(), ready)) {
            Ok(()) => handed.await.ok(),
            Err(_) => None,
        };

        let output = work.await;
        if let Some(back) = back {
            let _ = back.send(());
        }
        output
    }

    /// the checklist as the console holds it now
    #[cfg(test)]
    pub async fn checklist(&self) -> Result<Checklist> {
        let (reply, checklist) = oneshot::channel();
        self.requests
            .send(Request::Peek(reply))
            .map_err(|_| stopped())?;
        checklist.await.map_err(|_| stopped())
    }

    /// the closing summary, once the run ended
    pub async fn finish(&self, state: &StateDir, finale: &Finale) -> Result<()> {
        let (done, finished) = oneshot::channel();
        self.requests
            .send(Request::Finish(finale::lines(state, finale), done))
            .map_err(|_| stopped())?;
        finished.await.map_err(|_| stopped())
    }
}

fn stopped() -> eyre::Report {
    eyre!("the terminal's task stopped")
}

/// the terminal as the shell expects it back: out of raw mode, its cursor
/// shown
fn restore() {
    let _ = disable_raw_mode();
    let _ = execute!(std::io::stdout(), cursor::Show);
}

/// the terminal given back however the process ends early: a panic, or a
/// SIGTERM or SIGHUP, which leave at once as a second ctrl-c does. apart
/// from the terminal's task, which waits while ssh or sudo hold the
/// terminal and must not keep the process from ending then.
fn guard() -> Result<()> {
    static HOOK: Once = Once::new();
    HOOK.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            restore();
            previous(info);
        }));
    });

    let mut terminate = signal(SignalKind::terminate())?;
    let mut hangup = signal(SignalKind::hangup())?;
    tokio::spawn(async move {
        // 128 plus the signal's number, as a shell reports it
        let code = tokio::select! {
            _ = terminate.recv() => 128 + 15,
            _ = hangup.recv() => 128 + 1,
        };
        restore();
        std::process::exit(code);
    });
    Ok(())
}

/// the task's side: the model, where it goes, and the open question
struct Terminal {
    checklist: Checklist,
    out: Out,
    asking: Option<Asking>,
    cancelled: CancellationToken,
}

enum Out {
    /// `None` while the terminal is handed over
    Live(Option<Screen>),
    /// what the last printed lines said
    Plain { shown: Box<Checklist> },
}

struct Asking {
    reply: oneshot::Sender<Answer>,
    /// made for each prompt and dropped with it, so no thread reads the
    /// terminal while ssh or sudo use it
    keys: EventStream,
}

impl Terminal {
    const TICK: Duration = Duration::from_millis(100);

    async fn serve(
        mut self,
        mut requests: mpsc::UnboundedReceiver<Request>,
        mut interrupts: tokio::signal::unix::Signal,
    ) {
        let mut ticks = interval(Self::TICK);
        ticks.set_missed_tick_behavior(MissedTickBehavior::Skip);

        loop {
            let key = async {
                match &mut self.asking {
                    Some(asking) => asking.keys.next().await,
                    None => std::future::pending().await,
                }
            };

            tokio::select! {
                request = requests.recv() => match request {
                    Some(Request::Finish(lines, done)) => {
                        self.finish(lines);
                        let _ = done.send(());
                        return;
                    }
                    Some(request) => self.handle(request).await,
                    None => break,
                },
                Some(event) = key => {
                    if let Ok(Event::Key(key)) = event {
                        self.key(key);
                    }
                    self.draw();
                }
                _ = interrupts.recv() => self.interrupt(),
                _ = ticks.tick() => self.draw(),
            }
        }

        self.close_prompt();
    }

    async fn handle(&mut self, request: Request) {
        match request {
            Request::Row(row) => self.checklist.update(row),
            Request::Note(note) => self.checklist.notes.push(note),
            Request::Ask(prompt, reply) => {
                if enable_raw_mode().is_err() {
                    let _ = reply.send(Answer::Cancelled);
                    return;
                }
                self.checklist.prompt = Some(prompt);
                self.asking = Some(Asking {
                    reply,
                    keys: EventStream::new(),
                });
            }
            Request::HandOver(why, ready) => {
                self.hand_over(why, ready).await;
                return;
            }
            Request::Finish(..) => unreachable!("finish ends the task"),
            #[cfg(test)]
            Request::Peek(reply) => {
                let _ = reply.send(self.checklist.clone());
                return;
            }
        }
        if let Out::Plain { .. } = self.out {
            self.draw();
        }
    }

    fn key(&mut self, key: crossterm::event::KeyEvent) {
        let Some(prompt) = &mut self.checklist.prompt else {
            return;
        };
        let Some(answer) = prompt.key(key) else {
            return;
        };

        if let Answer::Cancelled = answer {
            self.cancelled.cancel();
        }
        if let Some(asking) = self.close_prompt() {
            let _ = asking.reply.send(answer);
        }
    }

    fn close_prompt(&mut self) -> Option<Asking> {
        self.checklist.prompt = None;
        let asking = self.asking.take()?;
        let _ = disable_raw_mode();
        Some(asking)
    }

    /// the first ctrl-c stops the run at the next step boundary; a second
    /// one leaves at once
    fn interrupt(&mut self) {
        if !self.cancelled.is_cancelled() {
            self.cancelled.cancel();
            return;
        }
        self.close_prompt();
        restore();
        std::process::exit(130);
    }

    async fn hand_over(&mut self, why: String, ready: oneshot::Sender<oneshot::Sender<()>>) {
        let live = matches!(self.out, Out::Live(_));
        if live {
            self.draw_still();
            self.out = Out::Live(None);
        } else {
            self.draw();
        }
        println!("{}", say::aside(why));
        let _ = std::io::stdout().flush();

        let (back, handed_back) = oneshot::channel();
        if ready.send(back).is_ok() {
            let _ = handed_back.await;
        }

        if live {
            println!();
            self.out = Out::Live(Screen::new().ok());
        }
    }

    fn finish(&mut self, lines: Vec<String>) {
        self.close_prompt();
        self.checklist.settle();
        match self.out {
            Out::Live(_) => {
                self.draw_still();
                self.out = Out::Live(None);
            }
            Out::Plain { .. } => self.draw(),
        }
        println!();
        for line in lines {
            println!("{line}");
        }
    }

    fn frame() -> Frame {
        let (width, height) = Screen::size();
        Frame {
            now: Timestamp::now(),
            width,
            height,
        }
    }

    fn draw(&mut self) {
        match &mut self.out {
            Out::Live(Some(screen)) => {
                let _ = screen.draw(&self.checklist.lines(Self::frame()));
            }
            Out::Live(None) => {}
            Out::Plain { shown } => {
                let now = Timestamp::now();
                for line in checklist::plain(shown, &self.checklist, now) {
                    println!("{line}");
                }
                **shown = self.checklist.clone();
            }
        }
    }

    fn draw_still(&mut self) {
        if let Out::Live(Some(screen)) = &mut self.out {
            let _ = screen.draw(&self.checklist.still(Self::frame()));
        }
    }
}
