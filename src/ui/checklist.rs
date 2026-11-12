//! the bootstrap's view: the checklist of steps, a model in and lines out

// the bootstrap's ui unit fills this module in; until then its
// signatures have unused parameters
#![allow(unused_variables)]

use super::{Frame, ask::Prompt};
use crate::{
    bootstrap::Step,
    common::model::{JobView, Timestamp},
};

pub struct Checklist {
    /// `coeus bootstrap nova ← nixos@192.168.1.40`
    pub title: String,
    /// `~/GitHub/nix · coeus/bootstrap/nova`, and how the last run ended
    pub subtitle: Vec<String>,
    pub rows: Vec<Row>,
    /// lines kept above the rows, such as a moved `/etc` file
    pub notes: Vec<String>,
    /// the open prompt, drawn under the active row
    pub prompt: Option<Prompt>,
}

#[derive(Clone, Debug)]
pub struct Row {
    pub step: Step,
    pub mark: Mark,
    /// what was found or is happening, after the label
    pub detail: String,
    /// lines under the row while it is active
    pub live: Vec<Live>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    /// `·` not reached yet
    Pending,
    /// a spinner
    Active,
    /// `✓ … already`: found done by looking
    Kept,
    /// `✓` done in this run
    Done,
    /// not shown: `nix` on NixOS, `reboot` when not needed
    Skipped,
    /// `⊘` waiting on a person
    Blocked,
    /// `✗`
    Failed,
}

#[derive(Clone, Debug)]
pub enum Live {
    /// the switch's job, drawn with the deploy board's progress cells
    Job(Box<JobView>),
    /// a count over named items, such as rekeyed secrets
    Count {
        done: usize,
        total: usize,
        current: String,
    },
    /// waiting on something outside since `since`
    Waiting { since: Timestamp, on: String },
    /// the last lines of a process's output
    Tail(Vec<String>),
}

impl Checklist {
    pub fn new(title: String, subtitle: Vec<String>) -> Self {
        todo!()
    }

    pub fn row_mut(&mut self, step: Step) -> &mut Row {
        todo!()
    }

    /// every line fit to the frame
    pub fn lines(&self, frame: Frame) -> Vec<String> {
        todo!()
    }
}

/// without a terminal: one line per change of a row's mark or detail, and
/// progress at most every tenth of the work, as `deploy` prints
pub fn plain(before: &Checklist, after: &Checklist, now: Timestamp) -> Vec<String> {
    todo!()
}
