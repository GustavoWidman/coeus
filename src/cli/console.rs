//! the terminal during a bootstrap: the checklist redrawn in place, and
//! everything that needs the person (a choice, a password, a host key)
//! asked inside it, so the live region and the questions never fight over
//! the cursor. ssh and sudo prompt on the terminal themselves: for those the
//! console draws a last frame, hands the terminal over, and starts a new
//! frame below when they are done.

// the bootstrap's ui unit fills this module in
#![allow(dead_code, unused_variables)]

use std::future::Future;

use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

use crate::{
    bootstrap::{Finale, Step},
    ui::{
        ask::{Answer, Prompt},
        checklist::{Checklist, Mark, Row},
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
    Finish(Box<Finale>, oneshot::Sender<()>),
}

impl Console {
    /// starts the task that owns the terminal: a [crate::ui::Screen] when
    /// live, [crate::ui::checklist::plain] lines otherwise. raw mode is on
    /// only while a prompt is open, so ctrl-c stays a signal otherwise.
    pub fn start(live: bool, checklist: Checklist) -> eyre::Result<Self> {
        todo!()
    }

    pub fn update(&self, row: Row) {
        let _ = self.requests.send(Request::Row(row));
    }

    /// shorthand for a row with no live lines
    pub fn mark(&self, step: Step, mark: Mark, detail: impl Into<String>) {
        self.update(Row {
            step,
            mark,
            detail: detail.into(),
            live: Vec::new(),
        });
    }

    pub fn note(&self, line: impl Into<String>) {
        let _ = self.requests.send(Request::Note(line.into()));
    }

    /// the person's answer; off a terminal this fails, and the caller says
    /// which `--answer` or terminal would do
    pub async fn ask(&self, prompt: Prompt) -> eyre::Result<Answer> {
        todo!()
    }

    /// gives the terminal to `work` (ssh asking a login password, sudo
    /// asking for the admin's key), saying `why` first
    pub async fn hand_over<T>(&self, why: &str, work: impl Future<Output = T>) -> T {
        todo!()
    }

    /// the closing summary, once the run ended
    pub async fn finish(&self, finale: Finale) -> eyre::Result<()> {
        todo!()
    }
}
