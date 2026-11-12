//! the rows of the checklist, and how a run ends

use std::{fmt, path::PathBuf};

use crate::{
    bootstrap::{
        branch::{PullRequest, body},
        contract::RepoPath,
        fleet::Reboot,
    },
    common::model::{FdaStatus, Hostname, JobSummary},
};

/// the steps of a run, in the order it goes through them. each one looks at
/// the world, acts on the difference and looks again.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Step {
    /// ssh to the machine, pin its host key, learn what it is and install
    /// the session key; or find its coeusd already answering
    Reach,
    /// Determinate Nix on a Mac; never shown for NixOS
    Nix,
    /// the contract's questions, the machine's files and its own secrets,
    /// committed on the branch
    Enroll,
    /// the branch on the default branch, every secret encrypted to exactly
    /// its recipients
    Rekey,
    /// the branch pushed, a draft pull request open
    Push,
    /// the machine runs the branch's head
    Switch,
    /// its coeusd answers over the tailnet with the fleet key
    Answer,
    /// it boots what it runs; only when it runs a kernel it did not boot
    Reboot,
    /// the pull request says what happened and is ready for review
    Review,
    /// the session key is gone from root's `authorized_keys`
    Cleanup,
}

impl Step {
    pub const ALL: [Self; 10] = [
        Self::Reach,
        Self::Nix,
        Self::Enroll,
        Self::Rekey,
        Self::Push,
        Self::Switch,
        Self::Answer,
        Self::Reboot,
        Self::Review,
        Self::Cleanup,
    ];

    /// the row's label
    pub const fn title(self) -> &'static str {
        match self {
            Self::Reach => "reach",
            Self::Nix => "nix",
            Self::Enroll => "enroll",
            Self::Rekey => "rekey",
            Self::Push => "push",
            Self::Switch => "switch",
            Self::Answer => "answer",
            Self::Reboot => "reboot",
            Self::Review => "review",
            Self::Cleanup => "cleanup",
        }
    }
}

impl fmt::Display for Step {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.title())
    }
}

/// how a run ended
#[derive(Debug)]
pub enum Finale {
    /// every step is where it should be
    Done(Box<Evidence>),
    /// ctrl-c; whatever runs on the machine keeps running
    Detached { at: Step },
    /// the machine could not be reached
    Unreached { at: Step, error: eyre::Report },
    /// a step failed; running again retries it
    Failed { at: Step, error: eyre::Report },
    /// a step needs something only a person can do first
    Blocked { at: Step, blocker: Blocker },
}

/// what a person must do before a run can go on
#[derive(Clone, Debug)]
pub enum Blocker {
    /// this machine's identity is not among these secrets' recipients, so it
    /// is not an admin of the fleet
    NotAdmin { files: Vec<RepoPath> },
    /// secrets whose recipients coeus cannot tell, with why: a rekey would
    /// lose a recipient it does not know
    Unreadable { files: Vec<(RepoPath, String)> },
    /// a file the enrollment adds is on the default branch already: the
    /// machine is enrolled there
    Enrolled { files: Vec<RepoPath> },
    /// the enroll commit holds these files otherwise than the answers make
    /// them now: it was enrolled with other answers
    Reanswered { files: Vec<RepoPath> },
    /// changes in the private clone nobody committed
    Uncommitted { clone: PathBuf, files: Vec<String> },
    /// the enrollment no longer applies onto the base it goes to
    Conflict { clone: PathBuf, base: String },
    /// the first switch of this head failed; `--retry` orders it again
    SwitchFailed { job: Box<JobSummary> },
    /// the coeus the machine's configuration pins has no `coeusd adopt`
    AdoptMissing { version: String },
    /// root refuses the session key, and nix on the machine refuses what
    /// `user` ships, since it is no trusted user and sudo asks a password
    RootRefused { user: String },
    /// the machine is up but not on the tailnet
    OffTailnet,
    /// `gh` is missing, signed out or refused
    Forge(String),
    /// a question without a terminal to ask it on, or `--answer`
    Unanswered { id: String },
    /// a secret only a person at a terminal can type
    NeedsTerminal { secret: RepoPath },
}

/// a step returns a blocker as its error; the run finds it with
/// `downcast_ref` and ends [Finale::Blocked]
impl std::error::Error for Blocker {}

impl fmt::Display for Blocker {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotAdmin { .. } => f.write_str("this machine is not an admin of the fleet"),
            Self::Unreadable { files } => match files.as_slice() {
                [(one, _)] => write!(f, "coeus cannot tell who opens {one}"),
                files => write!(f, "coeus cannot tell who opens {} secrets", files.len()),
            },
            Self::Enrolled { .. } => f.write_str("the machine is enrolled on the default branch"),
            Self::Reanswered { .. } => f.write_str("it was enrolled with other answers"),
            Self::Uncommitted { .. } => f.write_str("the clone has uncommitted changes"),
            Self::Conflict { .. } => f.write_str("the rebase stopped on a conflict"),
            Self::SwitchFailed { job } => write!(f, "the first switch, job {}, failed", job.id),
            Self::AdoptMissing { version } => write!(f, "coeus {version} has no `coeusd adopt`"),
            Self::RootRefused { user } => write!(f, "root refuses the key, {user} cannot ship"),
            Self::OffTailnet => f.write_str("the machine is not on the tailnet"),
            Self::Forge(reason) => write!(f, "gh: {reason}"),
            Self::Unanswered { id } => write!(f, "question {id} needs an answer"),
            Self::NeedsTerminal { secret } => write!(f, "{secret} needs a terminal"),
        }
    }
}

/// what the closing summary says after a run that got everywhere
#[derive(Debug)]
pub struct Evidence {
    pub host: Hostname,
    pub pull_request: PullRequest,
    /// what the pull request's coeus section says
    pub section: body::Evidence,
    /// what the contract says only a person can do
    pub notes: Vec<String>,
    pub reboot: Reboot,
    /// a Mac whose coeusd lacks Full Disk Access needs `coeus doctor --fix`
    pub full_disk_access: FdaStatus,
}
