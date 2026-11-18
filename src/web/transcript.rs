//! a job's log as the dashboard shows it: the lines `coeus logs` prints, plus
//! each builder's output under its derivation's name, every line typed by
//! what it says and stripped of terminal escapes. nix's own JSON never
//! reaches the browser.

use std::collections::HashMap;

use super::view::{LineKind, LogLine};
use crate::{
    common::model::{AttachFrame, JobEvent, Stamped},
    nix::{Activity, ActivityId, Event, Level, Line, strip_ansi},
    ui::{diff, format},
};

/// reads one job log from start to end, or from wherever a reader resumed
#[derive(Debug, Default)]
pub struct Transcript {
    /// the derivation each running build activity builds, to name the
    /// builder of an output line. empty after resuming mid-build, so those
    /// lines go unnamed.
    builders: HashMap<ActivityId, String>,
}

impl Transcript {
    pub fn lines(&mut self, frames: &[AttachFrame]) -> Vec<LogLine> {
        let mut lines = Vec::new();
        for frame in frames {
            match frame {
                AttachFrame::Event(stamped) => lines.extend(event(stamped)),
                AttachFrame::Nix(bytes) => {
                    for raw in String::from_utf8_lossy(bytes).lines() {
                        self.nix(raw, &mut lines);
                    }
                }
            }
        }
        lines
    }

    fn nix(&mut self, raw: &str, lines: &mut Vec<LogLine>) {
        let event = match Line::parse(raw) {
            Line::Nix(event) => event,
            Line::Output(text) => return push(lines, LineKind::Output, text, None),
        };

        match event {
            Event::Start(start) => {
                if let Some(drv) = start.drv_path() {
                    let name = format::derivation(&drv).to_owned();
                    self.builders.insert(start.id, name);
                }
                if start.level <= Level::Info {
                    push(lines, activity(start.activity), &start.text, None);
                }
            }
            Event::Stop(id) => {
                self.builders.remove(&id);
            }
            Event::Result(report) => {
                if let Some(text) = report.log_line() {
                    let builder = self.builders.get(&report.id).cloned();
                    push(lines, LineKind::Output, &text, builder);
                }
            }
            Event::Msg(message) if message.level <= Level::Info => {
                let kind = match message.level {
                    Level::Error => LineKind::Error,
                    _ => LineKind::Note,
                };
                for text in message.text.lines() {
                    push(lines, kind, text, None);
                }
            }
            Event::Msg(_) => {}
        }
    }
}

/// what nix starts, as the dashboard styles it
fn activity(activity: Activity) -> LineKind {
    match activity {
        Activity::Build => LineKind::Build,
        Activity::CopyPath
        | Activity::CopyPaths
        | Activity::FileTransfer
        | Activity::Substitute
        | Activity::FetchTree => LineKind::Fetch,
        _ => LineKind::Note,
    }
}

/// adds `text` as a line unless it is blank once its escapes are gone
fn push(lines: &mut Vec<LogLine>, kind: LineKind, text: &str, prefix: Option<String>) {
    let text = strip_ansi(text);
    let text = text.trim_end();
    if !text.trim().is_empty() {
        lines.push(LogLine {
            at: None,
            kind,
            text: text.to_owned(),
            prefix,
        });
    }
}

/// a coeus event as a line, or none for events that are only numbers
fn event(stamped: &Stamped<JobEvent>) -> Option<LogLine> {
    let (kind, text) = match &stamped.event {
        JobEvent::Started {
            attempt, version, ..
        } => {
            let attempt = match attempt {
                0 | 1 => String::new(),
                attempt => format!(", attempt {attempt}"),
            };
            (
                LineKind::Coeus,
                format!("runner started (coeus {version}{attempt})"),
            )
        }
        JobEvent::Phase(phase) => (LineKind::Phase, phase.as_str().to_owned()),
        JobEvent::Evaluated { toplevel, .. } => {
            (LineKind::Coeus, format!("system {}", toplevel.name()))
        }
        JobEvent::Progress(_) => return None,
        JobEvent::Diff(changes) if changes.is_empty() => {
            (LineKind::Coeus, "no package changes".to_owned())
        }
        JobEvent::Diff(changes) => (
            LineKind::Coeus,
            strip_ansi(&diff::totals(changes)).into_owned(),
        ),
        JobEvent::Note(note) => (LineKind::Note, note.clone()),
        JobEvent::Finished(outcome) => match outcome.is_success() {
            true => (LineKind::Done, outcome.to_string()),
            false => (LineKind::Error, outcome.to_string()),
        },
    };

    Some(LogLine {
        at: Some(stamped.at),
        kind,
        text,
        prefix: None,
    })
}
