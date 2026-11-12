use std::{collections::HashMap, time::Duration};

use super::{Entry, Mode, Retarget, Tail};
use crate::{
    client::FleetEvent,
    common::model::{Hostname, JobId, Timestamp},
    ui::{
        Board, Heading,
        board::{Row, Tally},
    },
};

/// everything `coeus top` shows, and nothing about the terminal it shows it on
pub struct Top {
    pub board: Board,
    tags: HashMap<Hostname, Vec<String>>,
    /// kept by name so it survives filtering and reordering
    selected: Option<Hostname>,
    /// matches names and tags; empty shows every machine
    pub filter: String,
    pub mode: Mode,
    notice: Option<Notice>,
    pub tail: Option<Tail>,
    /// the CLI's version, which the machines' checkups compare against
    pub version: &'static str,
    pacing: Pacing,
}

/// a transient line at the bottom: an answer, a refusal, an error
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notice {
    pub text: String,
    pub until: Timestamp,
}

/// redraws only when something changed, or a spinner or clock moved
#[derive(Clone, Copy, Debug)]
struct Pacing {
    dirty: bool,
    drawn_at: Option<Timestamp>,
}

impl Pacing {
    const ANIMATED: Duration = Duration::from_millis(100);
    const CLOCKS: Duration = Duration::from_secs(1);
}

impl Top {
    const NOTICE: Duration = Duration::from_secs(6);

    pub fn new(
        hosts: &[Hostname],
        tags: HashMap<Hostname, Vec<String>>,
        version: &'static str,
        now: Timestamp,
    ) -> Self {
        Self {
            board: Board::new(Heading::Watch { since: now }, hosts),
            tags,
            selected: hosts.first().cloned(),
            filter: String::new(),
            mode: Mode::Browse,
            notice: None,
            tail: None,
            version,
            pacing: Pacing {
                dirty: true,
                drawn_at: None,
            },
        }
    }

    pub fn apply(&mut self, event: FleetEvent) {
        self.board.apply(event);
        self.touch();
    }

    /// the machines the filter lets through, in inventory order
    pub fn visible(&self) -> impl Iterator<Item = &Row> {
        self.board.rows.iter().filter(|row| self.matches(row))
    }

    fn matches(&self, row: &Row) -> bool {
        let query = self.filter.trim().trim_start_matches('@').to_lowercase();
        if query.is_empty() {
            return true;
        }

        row.host.as_str().to_lowercase().contains(&query)
            || self.tags.get(&row.host).is_some_and(|tags| {
                tags.iter()
                    .any(|tag| tag.to_lowercase().starts_with(&query))
            })
    }

    /// the machine the detail and log panes are about: the chosen one while
    /// the filter shows it, else the first one shown
    pub fn selected(&self) -> Option<&Row> {
        let chosen = self
            .selected
            .as_ref()
            .and_then(|host| self.visible().find(|row| &row.host == host));
        chosen.or_else(|| self.visible().next())
    }

    /// where the selection sits among the visible machines
    pub fn selected_index(&self) -> Option<usize> {
        let selected = &self.selected()?.host;
        self.visible().position(|row| &row.host == selected)
    }

    /// moves the selection by `delta` visible machines, stopping at the ends
    pub fn step(&mut self, delta: isize) {
        let Some(at) = self.selected_index() else {
            return;
        };
        let last = self.visible().count().saturating_sub(1);
        let target = at.saturating_add_signed(delta).min(last);
        self.select_at(target);
    }

    pub fn select_at(&mut self, index: usize) {
        let host = self.visible().nth(index).map(|row| row.host.clone());
        if host.is_some() {
            self.selected = host;
            self.touch();
        }
    }

    pub fn notify(&mut self, text: impl Into<String>, now: Timestamp) {
        self.notice = Some(Notice {
            text: text.into(),
            until: Timestamp(now.0 + Self::NOTICE.as_millis() as i64),
        });
        self.touch();
    }

    pub fn notice(&self, now: Timestamp) -> Option<&Notice> {
        self.notice.as_ref().filter(|notice| notice.until > now)
    }

    /// keeps the log pane on the selected machine's running job. a job that
    /// just finished keeps its log on screen until the selection moves.
    pub fn retarget(&mut self) -> Option<Retarget> {
        let Some(row) = self.selected() else {
            return self.tail.take().map(|_| Retarget::Stop);
        };
        let host = row.host.clone();
        let running = row
            .view
            .as_ref()
            .and_then(|view| view.running.as_ref())
            .map(|job| job.summary.id);

        let same_host = self.tail.as_ref().is_some_and(|tail| tail.host == host);
        let same_job = same_host && self.tail.as_ref().map(|tail| tail.job) == running;

        match running {
            Some(_) if same_job => None,
            Some(job) => {
                self.tail = Some(Tail::new(host.clone(), job));
                self.touch();
                Some(Retarget::Follow { host, job })
            }
            None if same_host || self.tail.is_none() => None,
            None => {
                self.tail = None;
                self.touch();
                Some(Retarget::Stop)
            }
        }
    }

    /// lines of a followed job's log; ignored once the pane moved on
    pub fn tail_lines(&mut self, host: &Hostname, job: JobId, lines: Vec<String>, ended: bool) {
        let Some(tail) = self
            .tail
            .as_mut()
            .filter(|tail| &tail.host == host && tail.job == job)
        else {
            return;
        };
        tail.push(lines);
        tail.ended |= ended;
        self.touch();
    }

    /// running, done, failed and unreachable machines among the visible ones
    pub fn tally(&self) -> Tally {
        Tally::of(self.visible())
    }

    /// something changed and the next frame should show it
    pub const fn touch(&mut self) {
        self.pacing.dirty = true;
    }

    pub fn frame_due(&self, now: Timestamp) -> bool {
        let Some(drawn_at) = self.pacing.drawn_at else {
            return true;
        };
        let since = drawn_at.until(now);

        self.pacing.dirty
            || since >= Pacing::CLOCKS
            || (since >= Pacing::ANIMATED && self.animates(now))
    }

    pub fn drawn(&mut self, now: Timestamp) {
        if self.notice(now).is_none() {
            self.notice = None;
        }
        self.pacing = Pacing {
            dirty: false,
            drawn_at: Some(now),
        };
    }

    fn animates(&self, now: Timestamp) -> bool {
        self.notice.is_some() || self.visible().any(|row| Entry::of(row, now).animates())
    }
}
