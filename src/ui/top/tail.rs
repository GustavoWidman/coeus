use std::collections::VecDeque;

use crate::common::model::{Hostname, JobId};

/// the end of one job's log, as readable lines
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tail {
    pub host: Hostname,
    pub job: JobId,
    lines: VecDeque<String>,
    /// the job finished and its log is complete
    pub ended: bool,
}

/// what the dashboard's log pane should follow after the selection or a
/// machine's view changed
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Retarget {
    Follow { host: Hostname, job: JobId },
    Stop,
}

impl Tail {
    const KEPT: usize = 1000;

    pub const fn new(host: Hostname, job: JobId) -> Self {
        Self {
            host,
            job,
            lines: VecDeque::new(),
            ended: false,
        }
    }

    pub fn push(&mut self, lines: impl IntoIterator<Item = String>) {
        self.lines.extend(lines);
        let excess = self.lines.len().saturating_sub(Self::KEPT);
        self.lines.drain(..excess);
    }

    /// the newest `count` lines, oldest first
    pub fn last(&self, count: usize) -> impl Iterator<Item = &str> {
        let skip = self.lines.len().saturating_sub(count);
        self.lines.iter().skip(skip).map(String::as_str)
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }
}
