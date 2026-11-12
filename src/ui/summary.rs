use std::fmt::Write;

use colored::Colorize;

use super::{
    Board, Heading, Theme,
    board::{Row, Status, Verdict},
    describe, format,
    table::Table,
};
use crate::common::model::Timestamp;

/// how an order ended on every machine, for the terminal and for a GitHub
/// Actions step summary
pub struct Summary<'a> {
    board: &'a Board,
    now: Timestamp,
}

/// one machine's line of the summary, before any styling
struct Entry {
    host: String,
    outcome: String,
    source: String,
    duration: String,
    /// how to look closer, when there is a job to look at
    attach: Option<String>,
    success: Option<bool>,
}

impl<'a> Summary<'a> {
    pub const fn new(board: &'a Board, now: Timestamp) -> Self {
        Self { board, now }
    }

    pub fn lines(&self) -> Vec<String> {
        let mut table = Table::new(&["HOST", "OUTCOME", "SOURCE", "TOOK", "DETAILS"]);
        for entry in self.entries() {
            let outcome = match entry.success {
                Some(true) => entry.outcome.green().to_string(),
                Some(false) => entry.outcome.red().to_string(),
                None => entry.outcome.yellow().to_string(),
            };
            table.row(vec![
                Theme::host(&entry.host).to_string(),
                outcome,
                entry.source.cyan().to_string(),
                entry.duration,
                entry.attach.unwrap_or_default().dimmed().to_string(),
            ]);
        }

        let mut lines = table.lines();
        lines.push(String::new());
        lines.push(self.verdict());
        lines
    }

    /// `✓ 3 of 4 machines done in 4m12s, 1 failed`
    pub fn verdict(&self) -> String {
        let verdicts: Vec<Verdict> = self.board.verdicts().collect();
        let done = verdicts
            .iter()
            .filter(|verdict| **verdict == Verdict::Done)
            .count();
        let total = self.board.rows.len();
        let took = format::duration(self.board.heading.since().until(self.now));

        let problems = [
            (Verdict::Failed, "failed"),
            (Verdict::Rejected, "rejected"),
            (Verdict::Unreachable, "unreachable"),
            (Verdict::Held, "held back"),
        ]
        .into_iter()
        .filter_map(|(kind, label)| {
            let count = verdicts.iter().filter(|verdict| **verdict == kind).count();
            (count > 0).then(|| format!("{count} {label}"))
        })
        .collect::<Vec<_>>();

        match problems.is_empty() && done == total {
            true => format!("{} {done} of {total} done in {took}", "✓".green().bold()),
            false => format!(
                "{} {done} of {total} done in {took}, {}",
                "✗".red().bold(),
                problems.join(", ").red()
            ),
        }
    }

    /// a markdown table for `$GITHUB_STEP_SUMMARY`
    pub fn markdown(&self) -> String {
        let mut out = String::new();
        if let Heading::Order { source, action, .. } = &self.board.heading {
            let _ = writeln!(out, "### coeus {action} `{source}`\n");
        }
        out.push_str("| Host | Outcome | Source | Duration | Details |\n");
        out.push_str("| --- | --- | --- | --- | --- |\n");

        for entry in self.entries() {
            let mark = match entry.success {
                Some(true) => "✅",
                Some(false) => "❌",
                None => "⏳",
            };
            let _ = writeln!(
                out,
                "| {} | {mark} {} | `{}` | {} | {} |",
                entry.host,
                cell(&entry.outcome),
                entry.source,
                entry.duration,
                entry
                    .attach
                    .map(|attach| format!("`{attach}`"))
                    .unwrap_or_default(),
            );
        }

        out.push('\n');
        out
    }

    fn entries(&self) -> Vec<Entry> {
        self.board.rows.iter().map(|row| self.entry(row)).collect()
    }

    fn entry(&self, row: &Row) -> Entry {
        let fallback_source = match &self.board.heading {
            Heading::Order { source, .. } => source.to_string(),
            Heading::Watch { .. } => String::new(),
        };
        let summary = row.summary();

        Entry {
            host: row.host.to_string(),
            outcome: outcome(row.status()),
            source: summary.map_or(fallback_source, |summary| summary.source.to_string()),
            duration: summary
                .and_then(|summary| summary.duration_at(self.now))
                .map(format::duration)
                .unwrap_or_default(),
            attach: summary.map(|summary| format!("coeus attach {} {}", row.host, summary.id)),
            success: row.verdict().map(|verdict| verdict == Verdict::Done),
        }
    }
}

fn outcome(status: Status<'_>) -> String {
    match status {
        Status::Finished(_, outcome) => match outcome.failure() {
            Some(failure) => format!(
                "{} {}",
                Theme::outcome_label(outcome),
                describe::failure(failure)
            ),
            None => Theme::outcome_label(outcome),
        },
        Status::Rejected(rejection) => format!("rejected: {}", describe::rejection(rejection)),
        Status::Unreachable(reason) => format!("unreachable: {reason}"),
        Status::Held => "held back".into(),
        Status::Running(job) => format!(
            "still {}",
            job.summary
                .state
                .phase()
                .map_or("running", |phase| phase.as_str())
        ),
        Status::Queued(_) | Status::Admitted(_) => "still queued".into(),
        Status::Pending { .. } | Status::Sending => "not ordered".into(),
        Status::Connecting | Status::Idle => String::new(),
    }
}

/// markdown table cells cannot hold pipes or newlines
fn cell(text: &str) -> String {
    text.replace('|', "\\|").replace('\n', " ")
}
