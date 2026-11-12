//! the switch's job as the row shows it, from whichever door speaks

use crate::{
    common::model::{JobEvent, JobSummary, JobView, Stamped},
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
    pub fn parse(line: &str) -> Self {
        todo!()
    }
}

/// events over ssh are applied in order, nix lines folded into progress; a
/// view from coeusd replaces it whole, since coeusd read the same log
#[derive(Default)]
pub struct Feed {
    pub view: Option<JobView>,
    pub fold: LogFold,
    pub offset: u64,
}

impl Feed {
    pub fn line(&mut self, line: AdoptLine) {
        todo!()
    }

    pub fn replace(&mut self, view: JobView) {
        todo!()
    }

    pub fn finished(&self) -> Option<&JobSummary> {
        todo!()
    }
}
