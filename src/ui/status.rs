use colored::Colorize;

use super::{
    Board, Theme,
    board::{Row, Status},
    describe, format,
    table::Table,
};
use crate::{
    client::Checkup,
    common::model::{DaemonState, FdaStatus, JobState, MachineView, Phase, Timestamp},
};

/// one row per machine: what it does, what it runs, how its last job went,
/// whether it catches up on its own, and anything that needs a person
pub fn lines(board: &Board, now: Timestamp) -> Vec<String> {
    let mut table = Table::new(&["HOST", "STATE", "GEN", "RUNS", "LAST JOB", "CATCH-UP"]);

    for row in &board.rows {
        table.row(cells(row, now));
        for note in notes(row) {
            table.note(note);
        }
    }

    table.lines()
}

fn cells(row: &Row, now: Timestamp) -> Vec<String> {
    let host = Theme::host(row.host.as_str()).to_string();
    let Some(view) = &row.view else {
        return vec![host, state(row)];
    };
    let system = &view.system;

    vec![
        host,
        state(row),
        system
            .generation
            .map(|number| number.to_string())
            .unwrap_or_else(none),
        match (&system.deployed, &system.current) {
            (Some(source), _) => source.to_string().cyan().to_string(),
            (None, Some(current)) => current.name().dimmed().to_string(),
            (None, None) => none(),
        },
        last_job(view, now),
        catch_up(view),
    ]
}

fn state(row: &Row) -> String {
    let reconnecting = row.is_reconnecting();
    match row.status() {
        _ if reconnecting => format!("{} reconnecting", "↻".yellow().bold()),
        Status::Running(job) => {
            let phase = Theme::phase(job.summary.state.phase().unwrap_or(Phase::Preparing));
            match job.progress.fraction() {
                Some(fraction) => format!(
                    "{} {phase} {}",
                    "●".cyan().bold(),
                    format::percent(fraction)
                ),
                None => format!("{} {phase}", "●".cyan().bold()),
            }
        }
        Status::Queued(summary) => {
            format!("{} queued {}", "○".white(), summary.id.to_string().dimmed())
        }
        Status::Unreachable(_) => format!("{} {}", "⊘".red().bold(), "unreachable".red()),
        Status::Connecting => format!("{} connecting", "↻".bright_black()),
        _ => format!("{} {}", "·".bright_black(), "idle".dimmed()),
    }
}

/// `#12 succeeded 3m ago`
fn last_job(view: &MachineView, now: Timestamp) -> String {
    let Some(last) = view.recent.first() else {
        return none();
    };
    let JobState::Finished(outcome) = &last.state else {
        return none();
    };

    let when = last
        .finished_at
        .map(|finished| format::ago(finished, now))
        .unwrap_or_default();
    format!(
        "{} {} {}",
        last.id.to_string().dimmed(),
        Theme::outcome(outcome),
        when.dimmed()
    )
}

/// `deployed → caf4e2b`, or `holding deployed` while a manual deploy pauses it
fn catch_up(view: &MachineView) -> String {
    let Some(follow) = &view.system.follow else {
        return none();
    };

    match (follow.paused, &follow.latest) {
        (true, _) => format!("holding {}", follow.git_ref.bold())
            .yellow()
            .to_string(),
        (false, Some(latest)) => format!(
            "{} {} {}",
            follow.git_ref.bold(),
            "→".dimmed(),
            latest.rev.short().cyan()
        ),
        (false, None) => follow.git_ref.bold().to_string(),
    }
}

fn notes(row: &Row) -> Vec<String> {
    let mut notes = Vec::new();

    if let Status::Unreachable(reason) = row.status() {
        notes.push(reason.dimmed().to_string());
    }

    let Some(view) = &row.view else {
        return notes;
    };
    let system = &view.system;

    if view.daemon == DaemonState::Restarting {
        notes.push(
            "daemon restarting into a newer version"
                .yellow()
                .to_string(),
        );
    }
    if system.reboot_required {
        notes.push(format!(
            "{}: the running kernel differs from the current system's",
            "reboot required".yellow().bold()
        ));
    }
    if system.pending_boot {
        notes.push(format!(
            "{}: the next boot starts a newer generation than the running one",
            "boot pending".yellow().bold()
        ));
    }
    if system.fda == FdaStatus::Missing {
        notes.push(format!(
            "{}: {}",
            "Full Disk Access missing".red().bold(),
            describe::full_disk_access_fix(Checkup::LAUNCHER)
        ));
    }
    if let Some(last) = view.recent.first()
        && let JobState::Finished(outcome) = &last.state
        && let Some(failure) = outcome.failure()
    {
        notes.push(format!(
            "{} {}  {}",
            last.id,
            describe::failure(failure).red(),
            format!("→ coeus attach {} {}", row.host, last.id).dimmed()
        ));
    }

    notes
}

fn none() -> String {
    "—".dimmed().to_string()
}
