//! a job's log for people: coeus events as `[›] building` lines in the
//! style of the fleet's deploy scripts, and nix's own messages

use colored::Colorize;

use super::{Theme, describe, diff, format, say};
use crate::{
    common::model::{AttachFrame, ClosureDiff, JobEvent, JobSummary, Outcome, Stamped, Timestamp},
    nix::{Activity, Event, Line, strip_ansi},
};

/// a job event as one line, or none for events that are only numbers
pub fn event(stamped: &Stamped<JobEvent>) -> Option<String> {
    Some(match &stamped.event {
        JobEvent::Started {
            attempt, version, ..
        } => {
            let attempt = match attempt {
                0 | 1 => String::new(),
                attempt => format!(", attempt {attempt}"),
            };
            say::info(format!("runner started (coeus {version}{attempt})").dimmed())
        }
        JobEvent::Phase(phase) => say::info(Theme::phase(*phase)),
        JobEvent::Evaluated { toplevel, .. } => {
            say::info(format!("system {}", toplevel.name().cyan()))
        }
        JobEvent::Progress(_) => return None,
        JobEvent::Diff(changes) => say::info(diff::totals(changes)),
        JobEvent::Note(note) => say::aside(note.dimmed()),
        JobEvent::Finished(outcome) => finished(outcome),
    })
}

fn finished(outcome: &Outcome) -> String {
    let text = match outcome.failure() {
        Some(failure) => format!(
            "{} {}",
            Theme::outcome(outcome),
            describe::failure(failure).red()
        ),
        None => Theme::outcome(outcome).to_string(),
    };
    match outcome.is_success() {
        true => say::success(text),
        false => say::error(text),
    }
}

/// a frame of a job log as plain lines: events stamped with their time,
/// nix messages as nix printed them, and the derivations it starts building
pub fn frame(frame: &AttachFrame) -> Vec<String> {
    match frame {
        AttachFrame::Event(stamped) => event(stamped)
            .map(|line| format!("{} {line}", clock(stamped.at)))
            .into_iter()
            .collect(),
        AttachFrame::Nix(bytes) => String::from_utf8_lossy(bytes)
            .lines()
            .filter_map(nix_line)
            .collect(),
    }
}

fn nix_line(raw: &str) -> Option<String> {
    match Line::parse(raw) {
        Line::Nix(Event::Msg(message)) => Some(styled(message.text.trim_end())),
        Line::Nix(Event::Start(start)) if start.activity == Activity::Build => {
            let drv = start.drv_path()?;
            Some(
                format!("  building {}", format::derivation(&drv))
                    .dimmed()
                    .to_string(),
            )
        }
        Line::Nix(_) => None,
        Line::Output(text) if text.trim().is_empty() => None,
        Line::Output(text) => Some(styled(text)),
    }
}

/// nix's own colors, unless colors are off
fn styled(text: &str) -> String {
    match colored::control::SHOULD_COLORIZE.should_colorize() {
        true => text.to_owned(),
        false => strip_ansi(text).into_owned(),
    }
}

fn clock(at: Timestamp) -> String {
    at.to_local()
        .format("%H:%M:%S")
        .to_string()
        .dimmed()
        .to_string()
}

/// who ordered the job, what it deploys and when, above an attach
pub fn header(host: &str, job: &JobSummary, now: Timestamp) -> Vec<String> {
    vec![
        format!(
            "{} {} {} {}",
            "coeus".bold(),
            "›".dimmed(),
            Theme::host(host),
            format!("job {}", job.id).bold()
        ),
        format!(
            "  {} {} {} {} {}",
            job.action.as_str().bold(),
            job.source.to_string().cyan(),
            format!("by {}", job.origin).dimmed(),
            "·".dimmed(),
            format!(
                "ordered {} ({})",
                format::ago(job.ordered_at, now),
                job.ordered_at
            )
            .dimmed(),
        ),
    ]
}

/// how the job ended, how long it took, and what it changes
pub fn ending(
    host: &str,
    job: &JobSummary,
    outcome: &Outcome,
    changes: Option<&ClosureDiff>,
    limit: Option<usize>,
    now: Timestamp,
) -> Vec<String> {
    let took = job
        .duration_at(now)
        .map(|took| format!(" in {}", format::duration(took)))
        .unwrap_or_default();
    let mut lines = vec![format!(
        "{} {} {} {}{took}",
        Theme::outcome_glyph(outcome),
        Theme::host(host),
        format!("job {}", job.id).bold(),
        Theme::outcome(outcome),
    )];

    if let Some(failure) = outcome.failure() {
        lines.push(describe::failure(failure).red().to_string());
        lines.extend(
            failure
                .tail
                .iter()
                .map(|line| format!("  │ {line}").dimmed().to_string()),
        );
    }

    if let Some(changes) = changes {
        lines.push(String::new());
        lines.extend(diff::lines(changes, limit));
    }

    lines
}
