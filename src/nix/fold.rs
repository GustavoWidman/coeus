use std::collections::{HashMap, VecDeque};

use super::{Activity, ActivityId, Counts, Event, Level, Line, Report, Start, ansi};
use crate::common::model::{Failure, Phase, Progress, Tally};

/// folds a nix log, event by event, into what a deployment shows of it: the
/// [Progress] of its builds and downloads, whether evaluation is over, and the
/// last lines of output to explain a failure
#[derive(Debug, Default)]
pub struct LogFold {
    progress: Progress,
    changed: bool,
    evaluated: bool,
    /// the running activities progress is read from, by id
    tracked: HashMap<ActivityId, Tracked>,
    /// running builds, oldest first
    building: Vec<(ActivityId, String)>,
    tail: Tail,
    error: Option<String>,
}

#[derive(Debug)]
enum Tracked {
    /// the aggregate counting builds
    Builds,
    /// the aggregate counting substituted paths
    CopyPaths,
    /// one substituted path, parent of its download
    CopyPath,
    /// the download of a substituted path, with the bytes it reported so far
    Download { done: u64 },
}

impl LogFold {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn feed(&mut self, raw: &str) {
        self.apply(&Line::parse(raw));
    }

    pub fn apply(&mut self, line: &Line<'_>) {
        match line {
            Line::Output(text) => {
                self.tail.push(None, text);
            }
            Line::Nix(Event::Start(start)) => self.start(start),
            Line::Nix(Event::Stop(id)) => self.stop(*id),
            Line::Nix(Event::Result(report)) => self.report(report),
            Line::Nix(Event::Msg(msg)) if msg.level == Level::Error => self.record_error(&msg.text),
            Line::Nix(Event::Msg(_)) => {}
        }
    }

    pub const fn progress(&self) -> &Progress {
        &self.progress
    }

    /// the progress, if it changed since the last snapshot
    pub fn snapshot(&mut self) -> Option<Progress> {
        std::mem::take(&mut self.changed).then(|| self.progress.clone())
    }

    /// nix starts counting builds and downloads once it has evaluated
    /// everything it was asked for
    pub const fn evaluated(&self) -> bool {
        self.evaluated
    }

    /// the first line of the last error nix reported, without its `error:`
    pub fn last_error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// the last lines of output, oldest first, build lines prefixed by the
    /// name of their derivation as `nix build -L` does
    pub fn tail(&self) -> impl ExactSizeIterator<Item = &str> {
        self.tail.lines.iter().map(String::as_str)
    }

    pub fn failure(&self, phase: Phase) -> Failure {
        Failure {
            phase,
            message: self.last_error().unwrap_or("nix failed").to_owned(),
            tail: self.tail().map(str::to_owned).collect(),
        }
    }

    fn start(&mut self, start: &Start<'_>) {
        let tracked = match start.activity {
            Activity::Builds => Tracked::Builds,
            Activity::CopyPaths => Tracked::CopyPaths,
            Activity::CopyPath => Tracked::CopyPath,
            Activity::FileTransfer if self.is_copy_path(start.parent) => {
                Tracked::Download { done: 0 }
            }
            Activity::Build => {
                if let Some(path) = start.drv_path() {
                    self.building.push((start.id, drv_name(&path).to_owned()));
                    self.refresh_active();
                }
                return;
            }
            _ => return,
        };

        self.evaluated |= matches!(tracked, Tracked::Builds | Tracked::CopyPaths);
        self.tracked.insert(start.id, tracked);
    }

    fn stop(&mut self, id: ActivityId) {
        if self.tracked.remove(&id).is_some() {
            return;
        }
        if let Some(at) = self.building.iter().position(|(build, _)| *build == id) {
            self.building.remove(at);
            self.refresh_active();
        }
    }

    fn report(&mut self, report: &Report<'_>) {
        if let Some(counts) = report.progress() {
            self.count(report.id, counts);
        } else if let Some((Activity::FileTransfer, bytes)) = report.expected() {
            let changed = self.progress.expected_bytes != bytes;
            self.progress.expected_bytes = bytes;
            self.changed |= changed;
        } else if let Some(line) = report.log_line() {
            let name = self
                .building
                .iter()
                .find(|(build, _)| *build == report.id)
                .map(|(_, name)| name.as_str());
            self.tail.push(name, &line);
        }
    }

    fn count(&mut self, id: ActivityId, counts: Counts) {
        let tally = match self.tracked.get_mut(&id) {
            Some(Tracked::Builds) => &mut self.progress.builds,
            Some(Tracked::CopyPaths) => &mut self.progress.downloads,
            Some(Tracked::Download { done }) => {
                let delta = counts.done.saturating_sub(*done);
                *done = counts.done;
                self.progress.downloaded_bytes += delta;
                self.changed |= delta > 0;
                return;
            }
            Some(Tracked::CopyPath) | None => return,
        };

        let new = tally_of(counts);
        self.changed |= *tally != new;
        *tally = new;
    }

    fn record_error(&mut self, text: &str) {
        let mut kept = false;
        for line in text.lines() {
            kept |= self.tail.push(None, line);
        }
        if !kept {
            return;
        }

        let headline = ansi::strip_ansi(text.lines().next().unwrap_or_default());
        let headline = headline.trim();
        let headline = headline
            .strip_prefix("error:")
            .unwrap_or(headline)
            .trim_start();
        self.error = Some(headline.to_owned());
    }

    fn is_copy_path(&self, id: Option<ActivityId>) -> bool {
        id.is_some_and(|id| matches!(self.tracked.get(&id), Some(Tracked::CopyPath)))
    }

    fn refresh_active(&mut self) {
        let active = self
            .building
            .iter()
            .take(Progress::ACTIVE)
            .map(|(_, name)| name);
        if !active.clone().eq(self.progress.active.iter()) {
            self.progress.active = active.cloned().collect();
            self.changed = true;
        }
    }
}

/// the last [Failure::TAIL] lines worth showing, without terminal escapes
#[derive(Debug, Default)]
struct Tail {
    lines: VecDeque<String>,
}

impl Tail {
    /// keeps `text` unless it has nothing to read, and says whether it did
    fn push(&mut self, prefix: Option<&str>, text: &str) -> bool {
        let text = ansi::strip_ansi(text.trim_end());
        if !text.chars().any(char::is_alphanumeric) {
            return false;
        }

        let mut line = if self.lines.len() == Failure::TAIL {
            self.lines.pop_front().unwrap_or_default()
        } else {
            String::new()
        };
        line.clear();
        if let Some(prefix) = prefix {
            line.push_str(prefix);
            line.push_str("> ");
        }
        line.push_str(&text);

        self.lines.push_back(line);
        true
    }
}

fn tally_of(counts: Counts) -> Tally {
    let narrow = |count: u64| u32::try_from(count).unwrap_or(u32::MAX);
    Tally {
        done: narrow(counts.done),
        expected: narrow(counts.expected),
        running: narrow(counts.running),
        failed: narrow(counts.failed),
    }
}

/// `/nix/store/<hash>-hello-2.12.drv` is `hello-2.12`
fn drv_name(path: &str) -> &str {
    let file = path.rsplit('/').next().unwrap_or(path);
    let file = file.strip_suffix(".drv").unwrap_or(file);
    file.split_once('-').map_or(file, |(_, name)| name)
}
