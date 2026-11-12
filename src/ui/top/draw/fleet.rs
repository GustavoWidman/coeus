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

/// columns of the fleet pane, sized once for every machine shown. the gauge
/// and clock columns only appear while some machine fills them.
struct Columns {
    host: usize,
    gauge: bool,
    clock: bool,
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

    let entries: Vec<_> = top.visible().map(|row| Entry::of(row, now)).collect();
    if entries.is_empty() {
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
        gauge: entries.iter().any(|entry| entry.gauge.is_some()),
        clock: entries.iter().any(|entry| entry.clock.is_some()),
    };
    let width = usize::from(inner.width);
    let height = usize::from(inner.height);
    let selected = top.selected_index();
    let offset = selected.map_or(0, |at| at.saturating_sub(height.saturating_sub(1)));

    let lines = entries
        .iter()
        .enumerate()
        .skip(offset)
        .take(height)
        .map(|(at, entry)| {
            let chosen = Some(at) == selected;
            let line = columns.line(entry, chosen, now);
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

        let glyph = entry.row.glyph(entry.status, now);
        let mut cells = vec![pad(&host, self.host), pad(&label, Self::LABEL)];

        if self.gauge {
            let gauge = match entry.gauge {
                Some(fraction) => format!(
                    "{} {}",
                    Theme::bar(fraction, Self::BAR),
                    pad_left(&format::percent(fraction), 4).bold()
                ),
                None => String::new(),
            };
            cells.push(pad(&gauge, Self::BAR + 5));
        }
        if self.clock {
            let clock = pad_left(entry.clock.as_deref().unwrap_or_default(), Self::CLOCK);
            cells.push(match entry.status {
                Status::Running(_) => clock,
                _ => clock.dimmed().to_string(),
            });
        }
        cells.extend(entry.badges.iter().map(|badge| badge.label().to_string()));

        format!("{mark}{glyph} {}", cells.join("  "))
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
