//! the `Coeus-Bootstrap` trailer that marks the branch's commits, and the
//! log lines a run reads them back from

use eyre::{Result, eyre};

use crate::common::model::{CommitSha, Hostname};

pub(super) const KEY: &str = "Coeus-Bootstrap";

/// `git log` fields: sha, subject, the trailer's values; one record each
pub(super) const LOG_FORMAT: &str =
    "--format=%H%x1f%s%x1f%(trailers:key=Coeus-Bootstrap,valueonly,separator=%x1d)%x1e";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Trailer {
    Enroll,
    Rekey,
}

impl Trailer {
    fn word(self) -> &'static str {
        match self {
            Self::Enroll => "enroll",
            Self::Rekey => "rekey",
        }
    }

    /// `Coeus-Bootstrap: enroll nova`
    pub fn line(self, host: &Hostname) -> String {
        format!("{KEY}: {} {host}", self.word())
    }

    /// a trailer's value, when it marks this host's commit
    fn of(value: &str, host: &Hostname) -> Option<Self> {
        let (word, named) = value.trim().split_once(' ')?;
        if named.trim() != host.as_str() {
            return None;
        }
        [Self::Enroll, Self::Rekey]
            .into_iter()
            .find(|trailer| trailer.word() == word)
    }
}

/// one commit of the branch, as [LOG_FORMAT] prints it
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Marked {
    pub commit: CommitSha,
    pub subject: String,
    pub trailers: Vec<Trailer>,
}

impl Marked {
    pub fn is(&self, trailer: Trailer) -> bool {
        self.trailers.contains(&trailer)
    }
}

/// the commits of a `git log` printed with [LOG_FORMAT], newest first
pub(super) fn parse_log(text: &str, host: &Hostname) -> Result<Vec<Marked>> {
    text.split('\x1e')
        .map(|record| record.trim_start_matches('\n'))
        .filter(|record| !record.is_empty())
        .map(|record| {
            let mut fields = record.split('\x1f');
            let (Some(commit), Some(subject), Some(values)) =
                (fields.next(), fields.next(), fields.next())
            else {
                return Err(eyre!("unexpected git log record: {record:?}"));
            };
            Ok(Marked {
                commit: commit.parse()?,
                subject: subject.to_owned(),
                trailers: values
                    .split('\x1d')
                    .filter_map(|value| Trailer::of(value, host))
                    .collect(),
            })
        })
        .collect()
}
