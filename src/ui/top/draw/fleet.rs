use colored::{ColoredString, Colorize};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Style},
    widgets::Paragraph,
};

use super::{pane, quiet, text};
use crate::{
    common::model::{Phase, Timestamp},
    ui::{
        Theme,
        board::Status,
        format,
        text::{fit, pad, pad_left},
        top::{Entry, Top, ansi},
    },
};

/// columns of the fleet pane, sized once for every machine
struct Columns {
    host: usize,
}

impl Columns {
    const LABEL: usize = 12;
    const BAR: usize = 10;
    const CLOCK: usize = 7;
    const SELECTED: Color = Color::Indexed(236);
}

pub fn draw(frame: &mut Frame<'_>, area: Rect, top: &Top, now: Timestamp) {
    let block = pane(format!("{} {}", "machines".bold(), top.visible().count()));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let rows: Vec<_> = top.visible().collect();
    if rows.is_empty() {
        let line = quiet(format!("no machine matches /{}", top.filter));
        frame.render_widget(Paragraph::new(text(&[line], inner.width)), inner);
        return;
    }

    let columns = Columns {
        host: top
            .board
            .rows
            .iter()
            .map(|row| row.host.as_str().len())
            .max()
            .unwrap_or(0)
            .min(20),
    };
    let width = usize::from(inner.width);
    let height = usize::from(inner.height);
    let selected = top.selected_index();
    let offset = selected.map_or(0, |at| at.saturating_sub(height.saturating_sub(1)));

    let lines = rows
        .iter()
        .enumerate()
        .skip(offset)
        .take(height)
        .map(|(at, row)| {
            let chosen = Some(at) == selected;
            let line = columns.line(&Entry::of(row, now), chosen, now);
            let line = ansi::line(&pad(&fit(&line, width), width));
            match chosen {
                true => line.style(Style::new().bg(Columns::SELECTED)),
                false => line,
            }
        })
        .collect::<Vec<_>>();

    frame.render_widget(Paragraph::new(lines), inner);
}

impl Columns {
    fn line(&self, entry: &Entry<'_>, chosen: bool, now: Timestamp) -> String {
        let mark = match chosen {
            true => "▌".cyan().bold(),
            false => " ".normal(),
        };
        let host = Theme::host(&fit(entry.row.host.as_str(), self.host)).to_string();
        let label = fit(&label(entry).to_string(), Self::LABEL).into_owned();

        let gauge = match entry.gauge {
            Some(fraction) => format!(
                "{} {}",
                Theme::bar(fraction, Self::BAR),
                pad_left(&format::percent(fraction), 4).bold()
            ),
            None => String::new(),
        };
        let clock = entry.clock.as_deref().unwrap_or_default();
        let clock = match entry.status {
            Status::Running(_) => pad_left(clock, Self::CLOCK),
            _ => pad_left(clock, Self::CLOCK).dimmed().to_string(),
        };
        let badges = entry
            .badges
            .iter()
            .map(|badge| badge.label().to_string())
            .collect::<Vec<_>>()
            .join("  ");

        format!(
            "{mark}{} {}  {}  {}  {clock}  {badges}",
            entry.row.glyph(entry.status, now),
            pad(&host, self.host),
            pad(&label, Self::LABEL),
            pad(&gauge, Self::BAR + 5),
        )
        .trim_end()
        .to_owned()
    }
}

fn label(entry: &Entry<'_>) -> ColoredString {
    match entry.status {
        Status::Running(job) => Theme::phase(job.summary.state.phase().unwrap_or(Phase::Preparing)),
        Status::Queued(_) => "queued".white(),
        Status::Finished(_, outcome) => Theme::outcome(outcome),
        Status::Idle => "idle".bright_black(),
        Status::Connecting => "connecting".bright_black(),
        Status::Unreachable(_) => "unreachable".red(),
        Status::Rejected(_) => "rejected".red(),
        Status::Pending { .. } | Status::Held | Status::Sending | Status::Admitted(_) => {
            "ordered".white()
        }
    }
}
