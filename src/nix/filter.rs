use std::collections::HashMap;

use super::{Activity, ActivityId, Counts, Event, Level, Line, Report};

/// decides which lines of a nix log are worth keeping in a job log: all of
/// them but verbose messages and results that repeat what their activity last
/// reported, which change nothing for a reader such as `nom`
#[derive(Debug, Default)]
pub struct LogFilter {
    /// the last results of each running activity
    seen: HashMap<ActivityId, Seen>,
}

#[derive(Debug, Default)]
struct Seen {
    progress: Option<Counts>,
    expected: Vec<(Activity, u64)>,
}

impl LogFilter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn keep(&mut self, line: &Line<'_>) -> bool {
        match line {
            Line::Output(_) | Line::Nix(Event::Start(_)) => true,
            Line::Nix(Event::Msg(msg)) => msg.level < Level::Talkative,
            Line::Nix(Event::Stop(id)) => {
                self.seen.remove(id);
                true
            }
            Line::Nix(Event::Result(report)) => self.is_news(report),
        }
    }

    fn is_news(&mut self, report: &Report<'_>) -> bool {
        if let Some(counts) = report.progress() {
            let seen = self.seen.entry(report.id).or_default();
            seen.progress.replace(counts) != Some(counts)
        } else if let Some((activity, expected)) = report.expected() {
            let seen = self.seen.entry(report.id).or_default();
            match seen.expected.iter_mut().find(|(kind, _)| *kind == activity) {
                Some((_, last)) => std::mem::replace(last, expected) != expected,
                None => {
                    seen.expected.push((activity, expected));
                    true
                }
            }
        } else {
            true
        }
    }
}
