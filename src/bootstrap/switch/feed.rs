//! the switch's job as the row shows it, from whichever door speaks

use crate::{
    common::model::{JobEvent, JobSummary, JobView, Stamped},
    job::JobLog,
    nix::LogFold,
};

/// one line of `coeusd adopt --follow`
#[derive(Debug, PartialEq)]
pub enum AdoptLine {
    /// `@job <summary json>`, first: the job as it stands
    Job(Box<JobSummary>),
    /// `@coeus {"at":…,"event":…}`, as the job log holds it
    Event(Stamped<JobEvent>),
    /// `@nix <internal-json>`, as the job log holds it
    Nix(String),
    /// `@at <offset>`, where a follower resumes from
    At(u64),
    /// anything else, such as a line from a newer coeus
    Other,
}

impl AdoptLine {
    const JOB: &'static str = "@job ";
    const AT: &'static str = "@at ";

    pub fn parse(line: &str) -> Self {
        let line = line.trim_end_matches(['\r', '\n']);
        if let Some(json) = line.strip_prefix(Self::JOB) {
            return serde_json::from_str(json).map_or(Self::Other, |job| Self::Job(Box::new(job)));
        }
        if let Some(json) = line.strip_prefix(JobLog::COEUS) {
            return serde_json::from_str(json).map_or(Self::Other, Self::Event);
        }
        if line.starts_with(JobLog::NIX) {
            return Self::Nix(line.to_owned());
        }
        if let Some(offset) = line.strip_prefix(Self::AT) {
            return offset.trim().parse().map_or(Self::Other, Self::At);
        }
        Self::Other
    }
}

/// events over ssh are applied in order, nix lines folded into progress; a
/// view from coeusd replaces it whole, since coeusd read the same log. a
/// finished job stays finished: the events a stream replays after a
/// summary that says so are older than it
#[derive(Default)]
pub struct Feed {
    pub view: Option<JobView>,
    pub fold: LogFold,
    pub offset: u64,
}

impl Feed {
    pub fn line(&mut self, line: AdoptLine) {
        match line {
            AdoptLine::Job(summary) => match &mut self.view {
                // a follow resumed past the events already applied: only
                // what the summary says is newer
                Some(view) if view.summary.id == summary.id => view.summary = *summary,
                _ => self.view = Some(JobView::queued(*summary)),
            },
            AdoptLine::Event(event) => {
                if let Some(view) = &mut self.view
                    && !view.summary.state.is_finished()
                {
                    view.apply(&event);
                }
            }
            AdoptLine::Nix(line) => self.fold.feed(&line),
            AdoptLine::At(offset) => self.offset = offset,
            AdoptLine::Other => {}
        }
    }

    pub fn replace(&mut self, view: JobView) {
        self.view = Some(view);
    }

    pub fn finished(&self) -> Option<&JobSummary> {
        self.view
            .as_ref()
            .map(|view| &view.summary)
            .filter(|summary| summary.state.is_finished())
    }

    /// why the job failed, in nix's words when nix said
    pub fn failure(&self) -> Vec<String> {
        let mut lines: Vec<String> = self
            .fold
            .last_error()
            .map(str::to_owned)
            .into_iter()
            .collect();
        let tail: Vec<&str> = self.fold.tail().collect();
        lines.extend(
            tail[tail.len().saturating_sub(4)..]
                .iter()
                .map(|line| (*line).to_owned()),
        );
        lines
    }
}
