use colored::{ColoredString, Colorize};

use crate::common::model::{Outcome, Phase, Timestamp};

/// how each phase and outcome reads and looks, in one place
pub struct Theme;

impl Theme {
    const SPINNER: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
    const SPINNER_FRAME_MS: i64 = 80;
    const EIGHTHS: [char; 8] = [' ', '▏', '▎', '▍', '▌', '▋', '▊', '▉'];

    pub fn spinner(now: Timestamp) -> ColoredString {
        let frame = (now.0 / Self::SPINNER_FRAME_MS).rem_euclid(Self::SPINNER.len() as i64);
        Self::SPINNER[frame as usize].to_string().cyan().bold()
    }

    pub fn host(host: &str) -> ColoredString {
        host.magenta().bold()
    }

    pub fn phase(phase: Phase) -> ColoredString {
        let label = phase.as_str();
        match phase {
            Phase::Preparing => label.white(),
            Phase::Fetching => label.blue(),
            Phase::Evaluating => label.cyan(),
            Phase::Building => label.yellow(),
            Phase::Activating => label.magenta().bold(),
            Phase::Verifying => label.magenta(),
        }
    }

    /// `succeeded`, `failed`, `superseded by #12`, ...
    pub fn outcome_label(outcome: &Outcome) -> String {
        match outcome {
            Outcome::Failed(_) => "failed".into(),
            Outcome::RolledBack(_) => "rolled back".into(),
            outcome => outcome.to_string(),
        }
    }

    pub fn outcome(outcome: &Outcome) -> ColoredString {
        let label = Self::outcome_label(outcome);
        match outcome {
            Outcome::Succeeded => label.green().bold(),
            Outcome::UpToDate => label.green(),
            Outcome::Cancelled => label.yellow(),
            Outcome::Superseded { .. } => label.dimmed(),
            Outcome::Failed(_) | Outcome::Interrupted | Outcome::RolledBack(_) => {
                label.red().bold()
            }
        }
    }

    pub fn outcome_glyph(outcome: &Outcome) -> ColoredString {
        match outcome {
            Outcome::Succeeded | Outcome::UpToDate => "✓".green().bold(),
            Outcome::Superseded { .. } => "✓".dimmed(),
            Outcome::Cancelled => "✗".yellow().bold(),
            _ => "✗".red().bold(),
        }
    }

    /// a bar of `columns` cells filled to `fraction`, in eighths of a cell
    pub fn bar(fraction: f64, columns: usize) -> String {
        let eighths = (fraction.clamp(0.0, 1.0) * (columns * 8) as f64).round() as usize;
        let (full, partial) = (eighths / 8, eighths % 8);

        let mut filled = "█".repeat(full);
        if full < columns && partial > 0 {
            filled.push(Self::EIGHTHS[partial]);
        }
        let empty = columns - crate::ui::text::width(&filled);

        format!("{}{}", filled.green(), "─".repeat(empty).bright_black())
    }

    /// a bar with nothing to measure yet
    pub fn idle_bar(columns: usize) -> String {
        "─".repeat(columns).bright_black().to_string()
    }
}
