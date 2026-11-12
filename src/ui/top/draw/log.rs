use colored::Colorize;
use ratatui::{
    Frame,
    layout::Rect,
    widgets::{Padding, Paragraph},
};

use super::{pane, quiet, text};
use crate::ui::{Theme, top::Top};

pub fn draw(frame: &mut Frame<'_>, area: Rect, top: &Top) {
    let selected = top.selected().map(|row| &row.host);
    let tail = top
        .tail
        .as_ref()
        .filter(|tail| Some(&tail.host) == selected);

    let title = match tail {
        Some(tail) => format!(
            "{} {} {}  {}",
            "log".bold(),
            Theme::host(tail.host.as_str()),
            format!("job {}", tail.job).bold(),
            match tail.ended {
                true => "finished".dimmed(),
                false => "live".green(),
            }
        ),
        None => "log".bold().to_string(),
    };
    let block = pane(title).padding(Padding::horizontal(1));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let lines: Vec<String> = match (tail, selected) {
        (Some(tail), _) if tail.is_empty() => vec![quiet("waiting for the log…")],
        (Some(tail), _) => tail
            .last(usize::from(inner.height))
            .map(str::to_owned)
            .collect(),
        (None, Some(host)) => vec![quiet(format!("nothing running on {host}"))],
        (None, None) => Vec::new(),
    };
    frame.render_widget(Paragraph::new(text(&lines, inner.width)), inner);
}
