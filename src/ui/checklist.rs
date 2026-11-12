//! the bootstrap's view: the checklist of steps, a model in and lines out

use colored::{ColoredString, Colorize};

use super::{
    Frame, Theme,
    ask::Prompt,
    board::{Cells, extrapolate},
    format, say,
    text::{fit, pad, width},
};
use crate::{
    bootstrap::Step,
    common::model::{Hostname, JobState, JobView, Phase, Timestamp},
};

#[derive(Clone)]
pub struct Checklist {
    /// `coeus bootstrap nova ← nixos@192.168.1.40`, see [title]
    pub title: String,
    /// `~/GitHub/nix · coeus/bootstrap/nova`, and how the last run ended
    pub subtitle: Vec<String>,
    pub rows: Vec<Row>,
    /// lines kept under the rows, such as a moved `/etc` file
    pub notes: Vec<String>,
    /// the open prompt, drawn under the active row
    pub prompt: Option<Prompt>,
}

#[derive(Clone, Debug)]
pub struct Row {
    pub step: Step,
    pub mark: Mark,
    /// what was found or is happening, after the label
    pub detail: String,
    /// lines under the row while it is active, blocked or failed
    pub live: Vec<Live>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    /// `·` not reached yet
    Pending,
    /// a spinner
    Active,
    /// `✓ … already`: found done by looking
    Kept,
    /// `✓` done in this run
    Done,
    /// not shown: `nix` on NixOS, `reboot` when not needed
    Skipped,
    /// `⊘` waiting on a person
    Blocked,
    /// `✗`
    Failed,
}

#[derive(Clone, Debug)]
pub enum Live {
    /// the switch's job, drawn with the deploy board's progress cells
    Job(Box<JobView>),
    /// a count over named items, such as rekeyed secrets
    Count {
        done: usize,
        total: usize,
        current: String,
    },
    /// waiting on something outside since `since`
    Waiting { since: Timestamp, on: String },
    /// the last lines of a process's output
    Tail(Vec<String>),
}

/// `coeus bootstrap nova ← nixos@192.168.1.40`
pub fn title(host: &Hostname, destination: Option<&str>) -> String {
    let title = format!(
        "{} {} {}",
        "coeus".bold(),
        "bootstrap".bold(),
        Theme::host(host.as_str())
    );
    match destination {
        Some(destination) => format!("{title} {} {destination}", "←".dimmed()),
        None => title,
    }
}

/// the label column and the gap after it
const LABEL: usize = 9;
/// `  ✓ ` and the label: where details and live lines start
const INDENT: usize = 4 + LABEL;
/// a tail's last lines worth showing
const TAIL: usize = 6;

impl Row {
    pub fn new(step: Step, mark: Mark, detail: impl Into<String>) -> Self {
        Self {
            step,
            mark,
            detail: detail.into(),
            live: Vec::new(),
        }
    }

    fn glyph(&self, now: Timestamp) -> ColoredString {
        match self.mark {
            Mark::Pending | Mark::Skipped => "·".bright_black(),
            Mark::Active => Theme::spinner(now),
            Mark::Kept | Mark::Done => "✓".green().bold(),
            Mark::Blocked => "⊘".yellow().bold(),
            Mark::Failed => "✗".red().bold(),
        }
    }

    fn label(&self) -> ColoredString {
        match self.mark {
            Mark::Pending => self.step.title().bright_black(),
            Mark::Active => self.step.title().bold(),
            _ => self.step.title().normal(),
        }
    }

    fn styled_detail(&self) -> String {
        match self.mark {
            Mark::Pending => self.detail.dimmed().to_string(),
            Mark::Blocked => self.detail.yellow().to_string(),
            Mark::Failed => self.detail.red().to_string(),
            _ => self.detail.clone(),
        }
    }

    /// `  ✓ reach    NixOS 26.05 · x86_64-linux`, with `already` at
    /// `already_at` columns when the row was found done
    fn line(&self, frame: Frame, already_at: usize) -> String {
        let detail = self.styled_detail();
        if width(&detail) == 0 {
            return format!("  {} {}", self.glyph(frame.now), self.label());
        }
        let head = format!(
            "  {} {}",
            self.glyph(frame.now),
            pad(&self.label().to_string(), LABEL)
        );

        if self.mark != Mark::Kept {
            return format!("{head}{detail}");
        }
        let already = "already".dimmed();
        let room = frame.width.saturating_sub(INDENT + 2 + width("already"));
        let detail = fit(&detail, room);
        let column = already_at.clamp(width(&detail), room);
        format!("{head}{}  {already}", pad(&detail, column))
    }

    fn settled(&self) -> bool {
        matches!(self.mark, Mark::Pending | Mark::Kept | Mark::Done)
    }
}

impl Live {
    fn lines(&self, frame: Frame) -> Vec<String> {
        match self {
            Self::Job(job) => job_lines(job, frame),
            Self::Count {
                done,
                total,
                current,
            } => {
                let fraction = match total {
                    0 => 1.0,
                    total => *done as f64 / *total as f64,
                };
                vec![format!(
                    "{}  {done}/{total}  {}",
                    Theme::bar(fraction, frame.bar()),
                    current.dimmed()
                )]
            }
            Self::Waiting { since, on } => vec![format!(
                "{on}  {}",
                format::duration(since.until(frame.now)).dimmed()
            )],
            Self::Tail(lines) => lines[lines.len().saturating_sub(TAIL)..]
                .iter()
                .map(|line| format!("│ {line}").dimmed().to_string())
                .collect(),
        }
    }
}

/// a running job as a deploy row draws it, the build it is on beneath
fn job_lines(job: &JobView, frame: Frame) -> Vec<String> {
    let phase = match &job.summary.state {
        JobState::Running { phase } => *phase,
        JobState::Queued => return vec![format!("job {} queued", job.summary.id)],
        JobState::Finished(outcome) => {
            return vec![format!(
                "job {} {}",
                job.summary.id,
                Theme::outcome(outcome)
            )];
        }
    };

    let cells = Cells::of(job, extrapolate(job, frame.now), frame);
    let mut first = vec![
        pad(&Theme::phase(phase).to_string(), 10),
        cells.bar,
        cells.time,
    ];
    first.extend(cells.counts);

    let mut lines = vec![first.join("  ").trim_end().to_owned()];
    if phase == Phase::Building {
        lines.extend(cells.active);
    }
    lines
}

impl Checklist {
    /// every step pending; `nix` stays hidden until the machine is a Mac
    pub fn new(title: String, subtitle: Vec<String>) -> Self {
        let rows = Step::ALL
            .into_iter()
            .map(|step| match step {
                Step::Nix => Row::new(step, Mark::Skipped, ""),
                step => Row::new(step, Mark::Pending, ""),
            })
            .collect();

        Self {
            title,
            subtitle,
            rows,
            notes: Vec::new(),
            prompt: None,
        }
    }

    pub fn row(&self, step: Step) -> Option<&Row> {
        self.rows.iter().find(|row| row.step == step)
    }

    pub fn row_mut(&mut self, step: Step) -> &mut Row {
        let at = match self.rows.iter().position(|row| row.step == step) {
            Some(at) => at,
            None => {
                let at = self.rows.partition_point(|row| row.step < step);
                self.rows.insert(at, Row::new(step, Mark::Pending, ""));
                at
            }
        };
        &mut self.rows[at]
    }

    /// a row as a step reports it, replacing what it said before
    pub fn update(&mut self, row: Row) {
        let step = row.step;
        *self.row_mut(step) = row;
    }

    /// once the run ended: no spinner keeps turning and no prompt stays
    /// open. a step stopped halfway reads as not done.
    pub fn settle(&mut self) {
        self.prompt = None;
        for row in &mut self.rows {
            if row.mark == Mark::Active {
                row.mark = Mark::Pending;
                row.live.clear();
            }
        }
    }

    /// every line fit to the frame, with the ctrl-c hint while a step runs
    pub fn lines(&self, frame: Frame) -> Vec<String> {
        let mut lines = self.body(frame);
        if let Some(hint) = self.hint() {
            lines.push(String::new());
            lines.push(hint.dimmed().to_string());
        }
        self.fit(lines, frame)
    }

    /// the frame left in the scrollback when the terminal is handed over
    /// or the run ends: nothing on it asks for a key
    pub fn still(&self, frame: Frame) -> Vec<String> {
        let lines = self.body(frame);
        self.fit(lines, frame)
    }

    fn body(&self, frame: Frame) -> Vec<String> {
        let mut lines = vec![self.title.clone()];
        lines.extend(self.subtitle.iter().map(|line| line.dimmed().to_string()));
        lines.push(String::new());

        let already_at = self
            .rows
            .iter()
            .filter(|row| row.mark == Mark::Kept)
            .map(|row| width(&row.detail))
            .max()
            .unwrap_or(0);
        let asking_under = self.prompt.as_ref().map(|_| self.asking_row());
        let indent = " ".repeat(INDENT);

        for (at, row) in self.rows.iter().enumerate() {
            if row.mark == Mark::Skipped {
                continue;
            }
            lines.push(row.line(frame, already_at));
            if !row.settled() {
                let room = frame.width.saturating_sub(INDENT);
                lines.extend(row.live.iter().flat_map(|live| {
                    live.lines(Frame {
                        width: room,
                        ..frame
                    })
                    .into_iter()
                    .map(|line| format!("{indent}{line}"))
                }));
            }

            if let (Some(prompt), Some(under)) = (&self.prompt, asking_under)
                && under == at
            {
                lines.push(String::new());
                let room = frame.width.saturating_sub(2);
                lines.extend(
                    prompt
                        .lines(room)
                        .into_iter()
                        .map(|line| format!("  {line}")),
                );
                lines.push(String::new());
            }
        }

        if !self.notes.is_empty() {
            lines.push(String::new());
            lines.extend(self.notes.iter().map(say::aside));
        }
        lines
    }

    /// the row a prompt opens under: the active one, else the last one
    /// reached
    fn asking_row(&self) -> usize {
        let shown = |row: &&Row| row.mark != Mark::Skipped;
        self.rows
            .iter()
            .position(|row| row.mark == Mark::Active)
            .or_else(|| {
                self.rows
                    .iter()
                    .rposition(|row| shown(&row) && row.mark != Mark::Pending)
            })
            .unwrap_or(self.rows.len().saturating_sub(1))
    }

    /// what ctrl-c does now: during the switch the machine goes on alone
    fn hint(&self) -> Option<&'static str> {
        let active = self.rows.iter().find(|row| row.mark == Mark::Active)?;
        Some(match active.step {
            Step::Switch => {
                "ctrl-c detaches: the switch goes on on the machine, and running this again follows it"
            }
            Step::Reboot => {
                "ctrl-c detaches: the machine goes on rebooting, and running this again waits for it"
            }
            _ => "ctrl-c stops here, and running this again picks up where it left off",
        })
    }

    /// cut to the width; when taller than the terminal, the top goes first,
    /// since the live region cannot redraw what scrolled away
    fn fit(&self, lines: Vec<String>, frame: Frame) -> Vec<String> {
        let room = frame.height.saturating_sub(1).max(1);
        let skip = lines.len().saturating_sub(room);
        lines
            .into_iter()
            .skip(skip)
            .map(|line| fit(&line, frame.width).into_owned())
            .collect()
    }
}

/// without a terminal: one line per change of a row's mark or detail, a
/// note as it is added, and progress at most every tenth of the work, as
/// `deploy` prints
pub fn plain(before: &Checklist, after: &Checklist, now: Timestamp) -> Vec<String> {
    let time = now.to_local().format("%H:%M:%S").to_string().dimmed();
    let mut lines = Vec::new();

    for row in &after.rows {
        let earlier = before.row(row.step);
        let line = |text: String| format!("{time} {}  {text}", pad(row.step.title(), 7));
        lines.extend(plain_row(earlier, row).into_iter().map(line));
    }

    lines.extend(
        after
            .notes
            .iter()
            .skip(before.notes.len())
            .map(|note| format!("{time} {}", say::aside(note))),
    );
    lines
}

fn plain_row(before: Option<&Row>, row: &Row) -> Vec<String> {
    if row.mark == Mark::Skipped {
        return Vec::new();
    }
    let changed = before.is_none_or(|before| {
        (before.mark, before.detail.as_str()) != (row.mark, row.detail.as_str())
    });
    let mut texts = Vec::new();

    if changed && !(row.mark == Mark::Pending && row.detail.is_empty()) {
        let already = match row.mark {
            Mark::Kept => format!("  {}", "already".dimmed()),
            _ => String::new(),
        };
        texts.push(
            format!(
                "{} {}{already}",
                row.glyph(Timestamp(0)),
                row.styled_detail()
            )
            .trim_end()
            .to_owned(),
        );
        if matches!(row.mark, Mark::Blocked | Mark::Failed) {
            texts.extend(row.live.iter().flat_map(|live| {
                match live {
                    Live::Tail(lines) => lines
                        .iter()
                        .map(|line| format!("  │ {line}").dimmed().to_string())
                        .collect(),
                    _ => Vec::new(),
                }
            }));
        }
    }

    if row.mark == Mark::Active {
        let earlier = before.map(|before| before.live.as_slice()).unwrap_or(&[]);
        texts.extend(row.live.iter().filter_map(|live| plain_live(earlier, live)));
    }
    texts
}

/// a live line worth printing: a new phase, a new tenth of the work, or a
/// new thing waited on
fn plain_live(before: &[Live], live: &Live) -> Option<String> {
    match live {
        Live::Job(job) => {
            let phase = job.summary.state.phase()?;
            let fraction = job.progress.fraction();
            let seen = before.iter().find_map(|live| match live {
                Live::Job(job) => Some((job.summary.state.phase(), job.progress.fraction())),
                _ => None,
            });
            let fresh = match seen {
                Some((seen_phase, seen_fraction)) => {
                    seen_phase != Some(phase) || tenths(fraction) > tenths(seen_fraction)
                }
                None => true,
            };
            fresh.then(|| match fraction {
                Some(fraction) => format!(
                    "{} {} · {}",
                    Theme::phase(phase),
                    format::percent(fraction).bold(),
                    format::counts(&job.progress)
                ),
                None => format!("{} job {}", Theme::phase(phase), job.summary.id),
            })
        }
        Live::Count {
            done,
            total,
            current,
        } => {
            let fraction = |done: usize| Some(done as f64 / (*total).max(1) as f64);
            let seen = before.iter().find_map(|live| match live {
                Live::Count { done, .. } => Some(*done),
                _ => None,
            });
            let fresh = seen.is_none_or(|seen| tenths(fraction(*done)) > tenths(fraction(seen)));
            fresh.then(|| format!("{done}/{total} {}", current.dimmed()))
        }
        Live::Waiting { on, .. } => {
            let seen = before
                .iter()
                .any(|live| matches!(live, Live::Waiting { on: seen, .. } if seen == on));
            (!seen).then(|| on.clone())
        }
        Live::Tail(_) => None,
    }
}

fn tenths(fraction: Option<f64>) -> i64 {
    fraction.map_or(-1, |fraction| (fraction * 10.0).floor() as i64)
}
