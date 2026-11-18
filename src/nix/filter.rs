use std::collections::HashMap;

use super::{Activity, ActivityId, Counts, Event, Level, Line, Report};

/// decides which lines of a nix log are worth keeping in a job log: all of
/// them but verbose messages, results that repeat what their activity last
/// reported, and progress that has not moved a whole percent. a download
/// reports its bytes many times a second, and a log that kept every report
/// would be almost nothing else: a system rebuild's 87 MB, of which 97% was
/// progress. what a reader such as `nom` draws changes by a percent at most
#[derive(Debug, Default)]
pub struct LogFilter {
    /// the last results of each running activity
    seen: HashMap<ActivityId, Seen>,
}

#[derive(Debug, Default)]
struct Seen {
    /// the last progress written
    progress: Option<Counts>,
    /// the newest progress left out since, empty when there is none; written
    /// before the activity stops, so the log ends each activity where it did
    held: String,
    expected: Vec<(Activity, u64)>,
}

/// what goes into the job log for one line
#[derive(Debug, PartialEq, Eq)]
pub enum Kept {
    Nothing,
    Line,
    /// the activity's newest progress, left out until now, then the line
    After(String),
}

impl LogFilter {
    pub fn new() -> Self {
        Self::default()
    }

    /// what to write for `line`, parsed from `raw`
    pub fn keep(&mut self, raw: &str, line: &Line<'_>) -> Kept {
        let kept = match line {
            Line::Output(_) | Line::Nix(Event::Start(_)) => true,
            Line::Nix(Event::Msg(msg)) => msg.level < Level::Talkative,
            Line::Nix(Event::Stop(id)) => {
                return match self.seen.remove(id) {
                    Some(seen) if !seen.held.is_empty() => Kept::After(seen.held),
                    _ => Kept::Line,
                };
            }
            Line::Nix(Event::Result(report)) => self.is_news(raw, report),
        };
        if kept { Kept::Line } else { Kept::Nothing }
    }

    fn is_news(&mut self, raw: &str, report: &Report<'_>) -> bool {
        if let Some(counts) = report.progress() {
            let seen = self.seen.entry(report.id).or_default();
            let news = seen.progress.is_none_or(|last| moved(last, counts));
            seen.held.clear();
            if news {
                seen.progress = Some(counts);
            } else if seen.progress != Some(counts) {
                seen.held.push_str(raw);
            }
            news
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

/// whether progress from `last` to `now` shows: a whole percent further,
/// finished, or a change in what is expected, running or failed. without a
/// size there is no percent, and every change shows
fn moved(last: Counts, now: Counts) -> bool {
    let percent = |c: Counts| match c.expected {
        0 => c.done,
        expected => (u128::from(c.done.min(expected)) * 100 / u128::from(expected)) as u64,
    };
    (now.expected, now.running, now.failed) != (last.expected, last.running, last.failed)
        || (now.done >= now.expected) != (last.done >= last.expected)
        || percent(now) != percent(last)
}
