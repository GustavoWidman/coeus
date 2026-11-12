use std::time::Duration;

use colored::Colorize;

use super::Frame;
use crate::{
    common::model::JobView,
    ui::{
        Theme,
        format::{self, duration},
        text::{pad, pad_left},
    },
};

/// what a running job shows wherever it is drawn: the deploy board's rows
/// and a bootstrap's switch row look alike because both draw these
pub struct Cells {
    /// `██████▍─────  52%`, or an empty bar before nix says how much
    pub bar: String,
    /// `3m12s ~2m left`
    pub time: String,
    /// `812/1430 built · 1.2/3.4 GiB`
    pub counts: Option<String>,
    /// `linux-6.12.4 +3`, the build it is on
    pub active: Option<String>,
}

impl Cells {
    pub fn of(job: &JobView, left: Option<Duration>, frame: Frame) -> Self {
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
        let left = left
            .map(|left| format!(" ~{} left", duration(left)).dimmed().to_string())
            .unwrap_or_default();

        let counts = format::counts(&job.progress);
        let active = job.progress.active.first().map(|active| {
            let more = match job.progress.active.len() - 1 {
                0 => String::new(),
                more => format!(" +{more}"),
            };
            format!("{active}{more}").dimmed().to_string()
        });

        Self {
            bar,
            time: format!("{}{left}", pad(&elapsed, 6)),
            counts: (!counts.is_empty()).then_some(counts),
            active,
        }
    }
}
