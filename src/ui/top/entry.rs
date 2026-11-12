use colored::{ColoredString, Colorize};

use crate::{
    client::Link,
    common::model::{DaemonState, FdaStatus, MachineView, Phase, Source, Timestamp},
    ui::{
        board::{Row, Status},
        format,
    },
};

/// one machine's line in the fleet pane, before it is drawn
#[derive(Clone, Debug)]
pub struct Entry<'a> {
    pub row: &'a Row,
    pub status: Status<'a>,
    /// share of the build done, while building and nix has said how much
    /// there is
    pub gauge: Option<f64>,
    /// time running for a job in progress, age of a finished one
    pub clock: Option<String>,
    pub badges: Vec<Badge>,
}

/// something about a machine worth a glance besides its job
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Badge {
    /// the connection dropped; the view shown is the last one received
    Reconnecting,
    /// the machine stopped answering; the view shown is the last one received
    Unreachable,
    /// coeusd drains before exiting into a newer version
    Restarting,
    /// runs a kernel older than its system
    Reboot,
    /// a `boot` deployment waits for the next boot
    BootPending,
    /// a Mac whose runners cannot create or delete users
    NoDiskAccess,
    /// a deploy of something else paused catching up with this ref
    Holding(String),
    /// the followed ref moved past what runs; the machine catches up on its own
    Behind(String),
}

impl<'a> Entry<'a> {
    pub fn of(row: &'a Row, now: Timestamp) -> Self {
        let status = row.status();

        let gauge = match status {
            Status::Running(job) if job.summary.state.phase() == Some(Phase::Building) => {
                job.progress.fraction()
            }
            _ => None,
        };

        let clock = match status {
            Status::Running(job) => job
                .summary
                .started_at
                .map(|started| format::duration(started.until(now))),
            Status::Finished(summary, _) => summary
                .finished_at
                .map(|finished| format::ago(finished, now)),
            _ => None,
        };

        Self {
            row,
            status,
            gauge,
            clock,
            badges: Self::badges(row),
        }
    }

    fn badges(row: &Row) -> Vec<Badge> {
        let Some(view) = &row.view else {
            return Vec::new();
        };

        let mut badges = Vec::new();
        match &row.link {
            Link::Connected => {}
            Link::Connecting => badges.push(Badge::Reconnecting),
            Link::Unreachable(_) => badges.push(Badge::Unreachable),
        }
        if view.daemon == DaemonState::Restarting {
            badges.push(Badge::Restarting);
        }
        if view.system.reboot_required {
            badges.push(Badge::Reboot);
        } else if view.system.pending_boot {
            badges.push(Badge::BootPending);
        }
        if view.system.fda == FdaStatus::Missing {
            badges.push(Badge::NoDiskAccess);
        }
        badges.extend(Self::catch_up(view));
        badges
    }

    fn catch_up(view: &MachineView) -> Option<Badge> {
        let follow = view.system.follow.as_ref()?;
        if follow.paused {
            return Some(Badge::Holding(follow.git_ref.clone()));
        }

        let latest = follow.latest.as_ref()?;
        let current = matches!(
            &view.system.deployed,
            Some(Source::Git { rev, .. }) if *rev == latest.rev
        );
        (!current).then(|| Badge::Behind(follow.git_ref.clone()))
    }

    /// whether the line changes on its own while nothing new arrives: the
    /// spinner of a running job
    pub const fn animates(&self) -> bool {
        matches!(self.status, Status::Running(_))
    }
}

impl Badge {
    pub fn label(&self) -> ColoredString {
        match self {
            Self::Reconnecting => "↻ reconnecting".yellow(),
            Self::Unreachable => "⊘ unreachable".red(),
            Self::Restarting => "restarting".yellow(),
            Self::Reboot => "⏻ reboot".yellow().bold(),
            Self::BootPending => "⏻ boot pending".yellow(),
            Self::NoDiskAccess => "no disk access".red().bold(),
            Self::Holding(git_ref) => format!("holding {git_ref}").yellow(),
            Self::Behind(git_ref) => format!("behind {git_ref}").blue(),
        }
    }
}
