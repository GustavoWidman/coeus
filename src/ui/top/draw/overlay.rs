use colored::Colorize;
use ratatui::{
    Frame,
    layout::{Constraint, Flex, Layout, Rect},
    style::{Color, Style},
    widgets::{Clear, Paragraph},
};

use super::{pane, text};
use crate::ui::{
    text::{pad, width},
    top::Request,
};

pub fn confirm(frame: &mut Frame<'_>, request: &Request) {
    let question = request.question();
    let answers = format!(
        "{} {}   {} {}",
        "y".green().bold(),
        "yes".dimmed(),
        "n".red().bold(),
        "no".dimmed()
    );
    let lines = [
        String::new(),
        format!(" {question}"),
        String::new(),
        format!(" {answers}"),
    ];

    let columns = u16::try_from(width(&question) + 4)
        .unwrap_or(u16::MAX)
        .max(36);
    let area = centered(frame.area(), columns, lines.len() as u16 + 2);
    let border = match request.is_drastic() {
        true => Color::Red,
        false => Color::Yellow,
    };

    let block = pane("confirm".bold().to_string()).border_style(Style::new().fg(border));
    let inner = block.inner(area);
    frame.render_widget(Clear, area);
    frame.render_widget(block, area);
    frame.render_widget(Paragraph::new(text(&lines, inner.width)), inner);
}

pub fn help(frame: &mut Frame<'_>) {
    let keys = [
        ("↑ ↓  k j", "select a machine"),
        ("g G", "first, last machine"),
        ("⏎  a", "attach to its job through nom, then come back"),
        ("c", "cancel its running or queued job"),
        ("r", "roll it back to its previous generation"),
        ("R", "reboot it"),
        ("d", "deploy main to it and switch"),
        ("/", "filter by name or tag; esc clears"),
        ("?", "this help"),
        ("q  esc", "quit; deployments keep running"),
    ];
    let mut lines = vec![String::new()];
    lines.extend(
        keys.iter()
            .map(|(key, action)| format!("  {} {action}", pad(&key.bold().to_string(), 10))),
    );
    lines.push(String::new());
    lines.push(format!("  {}", "any key closes this".dimmed()));

    let area = centered(frame.area(), 62, lines.len() as u16 + 2);
    let block = pane("keys".bold().to_string()).border_style(Style::new().fg(Color::Cyan));
    let inner = block.inner(area);
    frame.render_widget(Clear, area);
    frame.render_widget(block, area);
    frame.render_widget(Paragraph::new(text(&lines, inner.width)), inner);
}

/// a box of at most `columns` by `rows` in the middle of `area`
fn centered(area: Rect, columns: u16, rows: u16) -> Rect {
    let [area] = Layout::horizontal([Constraint::Length(columns.min(area.width))])
        .flex(Flex::Center)
        .areas(area);
    let [area] = Layout::vertical([Constraint::Length(rows.min(area.height))])
        .flex(Flex::Center)
        .areas(area);
    area
}
