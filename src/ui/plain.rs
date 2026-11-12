use std::{collections::HashMap, time::Duration};

use colored::Colorize;

use super::{
    Board, Theme,
    board::{Row, Status},
    describe, format,
    text::pad,
};
use crate::common::model::{Hostname, JobId, JobSummary, Phase, Timestamp};

/// the board for output that cannot redraw, such as CI logs: one timestamped
/// line per change, and build progress at most every tenth of the work or
/// every half minute
#[derive(Default)]
pub struct Plain {
    seen: HashMap<Hostname, Seen>,
}

struct Seen {
    stage: Stage,
    reconnecting: bool,
    /// tenths of the build done at the last progress line
    tenths: u32,
    progress_at: Timestamp,
}

/// the part of a row's status worth a line when it changes
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    Pending,
    Held,
    Sending,
    Connecting,
    Unreachable,
    Rejected,
    Idle,
    Admitted(JobId),
    Queued(JobId),
    Running(JobId, Phase),
    Finished(JobId),
}

impl Stage {
    fn of(status: Status<'_>) -> Self {
        match status {
            Status::Pending { .. } => Self::Pending,
            Status::Held => Self::Held,
            Status::Sending => Self::Sending,
            Status::Connecting => Self::Connecting,
            Status::Unreachable(_) => Self::Unreachable,
            Status::Rejected(_) => Self::Rejected,
            Status::Idle => Self::Idle,
            Status::Admitted(job) => Self::Admitted(job),
            Status::Queued(summary) => Self::Queued(summary.id),
            Status::Running(job) => Self::Running(
                job.summary.id,
                job.summary.state.phase().unwrap_or(Phase::Preparing),
            ),
            Status::Finished(summary, _) => Self::Finished(summary.id),
        }
    }
}

impl Plain {
    const PROGRESS_EVERY: Duration = Duration::from_secs(30);

    /// the lines the board's changes since the last call deserve
    pub fn observe(&mut self, board: &Board, now: Timestamp) -> Vec<String> {
        let host_width = board
            .rows
            .iter()
            .map(|row| row.host.as_str().len())
            .max()
            .unwrap_or(0);
        let mut lines = Vec::new();

        for row in &board.rows {
            let line = |text: String| {
                format!(
                    "{} {}  {text}",
                    now.to_local().format("%H:%M:%S").to_string().dimmed(),
                    pad(&Theme::host(row.host.as_str()).to_string(), host_width),
                )
            };
            lines.extend(self.row(row, now).into_iter().map(line));
        }

        lines
    }

    fn row(&mut self, row: &Row, now: Timestamp) -> Vec<String> {
        let status = row.status();
        let stage = Stage::of(status);
        let reconnecting = row.is_reconnecting();
        let mut texts = Vec::new();

        let seen = self.seen.entry(row.host.clone()).or_insert_with(|| Seen {
            stage: Stage::Connecting,
            reconnecting: false,
            tenths: 0,
            progress_at: now,
        });

        if seen.stage != stage {
            texts.extend(change(status));
            seen.stage = stage;
            seen.tenths = 0;
            seen.progress_at = now;
        }

        if seen.reconnecting != reconnecting {
            texts.push(match reconnecting {
                true => "lost the connection, reconnecting".yellow().to_string(),
                false => "reconnected".dimmed().to_string(),
            });
            seen.reconnecting = reconnecting;
        }

        if let Status::Running(job) = status
            && let Some(fraction) = job.progress.fraction()
        {
            let tenths = (fraction * 10.0).floor() as u32;
            let due = seen.progress_at.until(now) >= Self::PROGRESS_EVERY;
            if tenths > seen.tenths || due {
                seen.tenths = tenths;
                seen.progress_at = now;
                texts.push(format!(
                    "{} {} · {}",
                    Theme::phase(job.summary.state.phase().unwrap_or(Phase::Building)),
                    format::percent(fraction).bold(),
                    format::counts(&job.progress)
                ));
            }
        }

        texts
    }
}

/// what to say when a row reaches `status`
fn change(status: Status<'_>) -> Vec<String> {
    let single = |text: String| vec![text];

    match status {
        Status::Connecting | Status::Sending => Vec::new(),
        Status::Pending { wave } => single(format!("waits for wave {wave}").dimmed().to_string()),
        Status::Held => single(
            "held back: the rollout stopped before this machine"
                .yellow()
                .to_string(),
        ),
        Status::Unreachable(reason) => single(format!("{} {}", "⊘ unreachable:".red(), reason)),
        Status::Rejected(rejection) => single(format!(
            "{} {}",
            "⊘ rejected:".red(),
            describe::rejection(rejection)
        )),
        Status::Idle => single("idle".dimmed().to_string()),
        Status::Admitted(job) => single(format!("ordered as job {job}")),
        Status::Queued(summary) => single(format!("{} {}", "queued".white(), provenance(summary))),
        Status::Running(job) => single(format!(
            "{} {}",
            Theme::phase(job.summary.state.phase().unwrap_or(Phase::Preparing)),
            provenance(&job.summary)
        )),
        Status::Finished(summary, outcome) => {
            let took = summary
                .duration()
                .map(|took| format!(" in {}", format::duration(took)))
                .unwrap_or_default();
            let mut texts = vec![format!(
                "{} {}{took} {}",
                Theme::outcome_glyph(outcome),
                Theme::outcome(outcome),
                provenance(summary)
            )];

            if let Some(failure) = outcome.failure() {
                texts.push(describe::failure(failure).red().to_string());
                texts.extend(
                    failure
                        .tail
                        .iter()
                        .map(|line| format!("  │ {line}").dimmed().to_string()),
                );
            }
            texts
        }
    }
}

/// `job #12 (main@caf4e2b by r3dlust@altar)`
fn provenance(summary: &JobSummary) -> String {
    format!(
        "job {} ({} by {})",
        summary.id, summary.source, summary.origin
    )
    .dimmed()
    .to_string()
}
