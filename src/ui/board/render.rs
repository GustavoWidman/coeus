use colored::{ColoredString, Colorize};

use super::{Board, Heading, Row, Status, Verdict};
use crate::{
    common::model::{JobSummary, JobView, MachineView, Outcome, Phase, Timestamp},
    ui::{
        Theme, describe,
        format::{self, duration},
        text::{fit, pad, pad_left},
    },
};

/// the terminal a board draws on, at one instant
#[derive(Clone, Copy, Debug)]
pub struct Frame {
    pub now: Timestamp,
    pub width: usize,
    pub height: usize,
}

impl Frame {
    const LABEL: usize = 11;
    /// header, two blank lines, footer and the line the cursor rests on
    const CHROME: usize = 5;

    /// the progress bar shrinks on narrow terminals before anything is cut
    fn bar(&self) -> usize {
        (self.width / 6).clamp(8, 24)
    }
}

impl Board {
    /// the whole board, every line cut to the frame's width and the rows to
    /// its height
    pub fn lines(&self, frame: Frame) -> Vec<String> {
        let host_width = self
            .rows
            .iter()
            .map(|row| row.host.as_str().len())
            .max()
            .unwrap_or(0);
        let room = frame.height.saturating_sub(Frame::CHROME).max(1);

        let mut lines = vec![self.header(frame), String::new()];
        let shown = match self.rows.len() > room {
            true => room - 1,
            false => self.rows.len(),
        };
        lines.extend(
            self.rows[..shown]
                .iter()
                .map(|row| row.line(self, host_width, frame)),
        );
        if shown < self.rows.len() {
            lines.push(
                format!("  … {} more machines", self.rows.len() - shown)
                    .dimmed()
                    .to_string(),
            );
        }
        lines.push(String::new());
        lines.push(self.footer());

        lines
            .into_iter()
            .map(|line| fit(&line, frame.width).into_owned())
            .collect()
    }

    fn header(&self, frame: Frame) -> String {
        let machines = match self.rows.len() {
            1 => "1 machine".to_owned(),
            count => format!("{count} machines"),
        };
        let elapsed = duration(self.heading.since().until(frame.now));

        match &self.heading {
            Heading::Order {
                id, source, action, ..
            } => format!(
                "{} {} {} {} {}  {}  {}",
                "coeus".bold(),
                action.as_str().bold(),
                source.to_string().cyan().bold(),
                "→".dimmed(),
                machines,
                format!("order {id}").dimmed(),
                elapsed.dimmed(),
            ),
            Heading::Watch { .. } => format!(
                "{} {} {}  {}",
                "coeus".bold(),
                "watching".bold(),
                machines,
                elapsed.dimmed()
            ),
        }
    }

    fn footer(&self) -> String {
        let mut tally = Tally::default();
        for row in &self.rows {
            tally.count(row);
        }

        let hint = match self.heading {
            Heading::Order { .. } => "ctrl-c detaches, deployments keep running",
            Heading::Watch { .. } => "ctrl-c stops watching, deployments keep running",
        };

        match tally.line() {
            line if line.is_empty() => hint.dimmed().to_string(),
            line => format!("{line}   {}", hint.dimmed()),
        }
    }
}

#[derive(Default)]
struct Tally {
    running: usize,
    waiting: usize,
    done: usize,
    failed: usize,
    unreached: usize,
}

impl Tally {
    fn count(&mut self, row: &Row) {
        match (row.status(), row.verdict()) {
            (Status::Running(_), _) => self.running += 1,
            (_, Some(Verdict::Done)) => self.done += 1,
            (_, Some(Verdict::Failed | Verdict::Held)) => self.failed += 1,
            (_, Some(Verdict::Rejected | Verdict::Unreachable)) | (Status::Unreachable(_), _) => {
                self.unreached += 1
            }
            (Status::Finished(_, outcome), None) if outcome.is_success() => self.done += 1,
            (Status::Finished(..), None) => self.failed += 1,
            (Status::Idle | Status::Connecting, _) => {}
            _ => self.waiting += 1,
        }
    }

    fn line(&self) -> String {
        [
            (self.running, "running".cyan()),
            (self.waiting, "waiting".white()),
            (self.done, "done".green()),
            (self.failed, "failed".red()),
            (self.unreached, "unreachable".yellow()),
        ]
        .into_iter()
        .filter(|(count, _)| *count > 0)
        .map(|(count, label)| format!("{} {label}", count.to_string().bold()))
        .collect::<Vec<_>>()
        .join(" · ")
    }
}

impl Row {
    fn line(&self, board: &Board, host_width: usize, frame: Frame) -> String {
        let status = self.status();
        let (label, detail) = self.cells(status, board, frame);

        format!(
            "{} {}  {}  {}",
            self.glyph(status, frame),
            pad(&Theme::host(self.host.as_str()).to_string(), host_width),
            pad(&label.to_string(), Frame::LABEL),
            detail
        )
        .trim_end()
        .to_owned()
    }

    fn glyph(&self, status: Status<'_>, frame: Frame) -> ColoredString {
        if self.is_reconnecting() {
            return "↻".yellow().bold();
        }

        match status {
            Status::Running(_) => Theme::spinner(frame.now),
            Status::Finished(_, outcome) => Theme::outcome_glyph(outcome),
            Status::Unreachable(_) => "⊘".red().bold(),
            Status::Rejected(_) => "⊘".yellow().bold(),
            Status::Connecting => "↻".bright_black(),
            Status::Idle => "·".bright_black(),
            Status::Pending { .. }
            | Status::Held
            | Status::Sending
            | Status::Admitted(_)
            | Status::Queued(_) => "○".white(),
        }
    }

    fn cells(&self, status: Status<'_>, board: &Board, frame: Frame) -> (ColoredString, String) {
        let watching = matches!(board.heading, Heading::Watch { .. });

        match status {
            Status::Pending { wave } => (
                "waiting".white(),
                format!("wave {wave}").dimmed().to_string(),
            ),
            Status::Held => (
                "held back".yellow(),
                "the rollout stopped before this machine"
                    .dimmed()
                    .to_string(),
            ),
            Status::Sending => ("ordering".white(), String::new()),
            Status::Connecting => ("connecting".bright_black(), String::new()),
            Status::Unreachable(reason) => ("unreachable".red(), reason.dimmed().to_string()),
            Status::Rejected(rejection) => (
                "rejected".red(),
                describe::rejection(rejection).yellow().to_string(),
            ),
            Status::Idle => ("idle".bright_black(), self.deployed()),
            Status::Admitted(job) => ("ordered".white(), format!("job {job}").dimmed().to_string()),
            Status::Queued(summary) => ("queued".white(), queued(summary, watching)),
            Status::Running(job) => (
                Theme::phase(job.summary.state.phase().unwrap_or(Phase::Preparing)),
                self.running(job, watching, frame),
            ),
            Status::Finished(summary, outcome) => (
                Theme::outcome(outcome),
                self.finished(summary, outcome, watching, frame),
            ),
        }
    }

    fn running(&self, job: &JobView, watching: bool, frame: Frame) -> String {
        let bar = match job.progress.fraction() {
            Some(fraction) => format!(
                "{} {}",
                Theme::bar(fraction, frame.bar()),
                pad_left(&format::percent(fraction), 4).bold()
            ),
            None => format!("{} {}", Theme::idle_bar(frame.bar()), " ".repeat(4)),
        };

        let elapsed = job
            .summary
            .started_at
            .map(|started| duration(started.until(frame.now)))
            .unwrap_or_default();
        let eta = self
            .view
            .as_ref()
            .and_then(|view| super::eta(view, job, frame.now))
            .map(|left| format!(" ~{} left", duration(left)).dimmed().to_string())
            .unwrap_or_default();

        let mut parts = vec![bar, format!("{}{eta}", pad(&elapsed, 6))];
        if watching {
            parts.push(provenance(&job.summary));
        }

        let counts = format::counts(&job.progress);
        if !counts.is_empty() {
            parts.push(counts);
        }
        if let Some(active) = job.progress.active.first() {
            let more = job.progress.active.len() - 1;
            let more = if more > 0 {
                format!(" +{more}")
            } else {
                String::new()
            };
            parts.push(format!("{active}{more}").dimmed().to_string());
        }

        parts.join("  ")
    }

    fn finished(
        &self,
        summary: &JobSummary,
        outcome: &Outcome,
        watching: bool,
        frame: Frame,
    ) -> String {
        let mut parts = Vec::new();
        if let Some(took) = summary.duration_at(frame.now) {
            parts.push(format!("in {}", duration(took)));
        }
        if watching {
            if let Some(finished) = summary.finished_at {
                parts.push(format::ago(finished, frame.now).dimmed().to_string());
            }
            parts.push(provenance(summary));
        }

        if let Some(failure) = outcome.failure() {
            parts.push(describe::failure(failure).red().to_string());
            parts.push(
                format!("→ coeus attach {} {}", self.host, summary.id)
                    .dimmed()
                    .to_string(),
            );
        }

        parts.join("  ")
    }

    /// what the machine runs, for an idle row
    fn deployed(&self) -> String {
        let Some(MachineView { system, .. }) = &self.view else {
            return String::new();
        };

        match (&system.deployed, system.generation) {
            (Some(source), Some(generation)) => format!("runs {source} (generation {generation})")
                .dimmed()
                .to_string(),
            (Some(source), None) => format!("runs {source}").dimmed().to_string(),
            (None, Some(generation)) => format!("generation {generation}").dimmed().to_string(),
            (None, None) => String::new(),
        }
    }
}

fn queued(summary: &JobSummary, watching: bool) -> String {
    let job = format!("job {}", summary.id).dimmed().to_string();
    match watching {
        true => format!("{job}  {}", provenance(summary)),
        false => job,
    }
}

/// `#12 main@caf4e2b by github deploy #4211`
fn provenance(summary: &JobSummary) -> String {
    format!(
        "{} {} {}",
        summary.id.to_string().dimmed(),
        summary.source.to_string().cyan(),
        format!("by {}", summary.origin).dimmed()
    )
}
