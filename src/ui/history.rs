use colored::Colorize;

use super::{Theme, describe, format, table::Table};
use crate::common::model::{Generation, Hostname, JobState, JobSummary, Timestamp};

/// jobs of one or more machines, newest first, failures explained under
/// their row. the host column only appears for more than one machine.
pub fn jobs(jobs: &[(Hostname, JobSummary)], now: Timestamp) -> Vec<String> {
    let several = jobs.iter().any(|(host, _)| host != &jobs[0].0);
    let mut header = vec!["JOB", "STATE", "ACTION", "SOURCE", "BY", "WHEN", "TOOK"];
    if several {
        header.insert(0, "HOST");
    }

    let mut table = Table::new(&header);
    for (host, job) in jobs {
        let mut cells = vec![
            job.id.to_string().dimmed().to_string(),
            state(&job.state),
            job.action.to_string(),
            job.source.to_string().cyan().to_string(),
            job.origin.to_string().dimmed().to_string(),
            format::ago(job.ordered_at, now),
            job.duration_at(now)
                .map(format::duration)
                .unwrap_or_default(),
        ];
        if several {
            cells.insert(0, Theme::host(host.as_str()).to_string());
        }
        table.row(cells);

        if let JobState::Finished(outcome) = &job.state
            && let Some(failure) = outcome.failure()
        {
            table.note(describe::failure(failure).red().to_string());
        }
    }

    table.lines()
}

fn state(state: &JobState) -> String {
    match state {
        JobState::Queued => "queued".white().to_string(),
        JobState::Running { phase } => format!("{} {}", "●".cyan(), Theme::phase(*phase)),
        JobState::Finished(outcome) => format!(
            "{} {}",
            Theme::outcome_glyph(outcome),
            Theme::outcome(outcome)
        ),
    }
}

/// the system profile's generations, newest first, the running and booted
/// ones marked
pub fn generations(generations: &[Generation], now: Timestamp) -> Vec<String> {
    let mut table = Table::new(&["GEN", "", "CREATED", "", "SYSTEM"]);
    for generation in generations {
        let marks = [
            generation
                .current
                .then(|| "current".green().bold().to_string()),
            generation.booted.then(|| "booted".blue().to_string()),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" ");

        table.row(vec![
            generation.number.to_string().bold().to_string(),
            marks,
            format::ago(generation.created_at, now),
            generation.created_at.to_string().dimmed().to_string(),
            generation.toplevel.name().dimmed().to_string(),
        ]);
    }
    table.lines()
}
