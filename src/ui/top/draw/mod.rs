//! the dashboard on a ratatui frame. every pane is built as styled lines,
//! like the rest of the CLI's views, and read into ratatui text at the end.

mod detail;
mod fleet;
mod log;
mod overlay;

use colored::Colorize;
use ratatui::{
    Frame,
    layout::{Alignment, Rect},
    style::{Color, Style},
    text::Line,
    widgets::{Block, BorderType, Paragraph},
};

use super::{Mode, Panes, Top, ansi};
use crate::{
    common::model::Timestamp,
    ui::{say, text::fit},
};

pub fn draw(frame: &mut Frame<'_>, top: &Top, now: Timestamp) {
    let panes = Panes::of(frame.area(), top.visible().count());

    header(frame, panes.header, top, now);
    fleet::draw(frame, panes.fleet, top, now);
    detail::draw(frame, panes.detail, top, now);
    log::draw(frame, panes.log, top);
    footer(frame, panes.footer, top, now);

    match &top.mode {
        Mode::Confirm(request) => overlay::confirm(frame, request),
        Mode::Help => overlay::help(frame),
        Mode::Browse | Mode::Filter => {}
    }
}

fn header(frame: &mut Frame<'_>, area: Rect, top: &Top, now: Timestamp) {
    let shown = top.visible().count();
    let machines = match (shown, top.board.rows.len()) {
        (1, 1) => "1 machine".to_owned(),
        (shown, total) if shown == total => format!("{total} machines"),
        (shown, total) => format!("{shown} of {total} machines"),
    };

    let tally = top.tally().line();
    let mut left = format!(" {} {}  {machines}", "coeus".bold(), "top".bold());
    if !tally.is_empty() {
        left.push_str(&format!("  {}  {tally}", "·".dimmed()));
    }
    if !top.filter.is_empty() {
        left.push_str(&format!("  {}", format!("/{}", top.filter).cyan()));
    }

    let clock = format!("{} ", now.to_local().format("%H:%M:%S"))
        .dimmed()
        .to_string();
    frame.render_widget(
        Paragraph::new(text(&[left], area.width)),
        Rect {
            width: area.width.saturating_sub(10),
            ..area
        },
    );
    frame.render_widget(
        Paragraph::new(ansi::line(&clock)).alignment(Alignment::Right),
        area,
    );
}

fn footer(frame: &mut Frame<'_>, area: Rect, top: &Top, now: Timestamp) {
    let line = match (&top.mode, top.notice(now)) {
        (Mode::Filter, _) => format!(
            " {}{}{}   {}",
            "/".cyan().bold(),
            top.filter,
            "▏".cyan(),
            "names or tags · enter keeps · esc clears".dimmed()
        ),
        (_, Some(notice)) => format!(" {}", notice.text),
        _ => hints(),
    };
    frame.render_widget(Paragraph::new(text(&[line], area.width)), area);
}

fn hints() -> String {
    let keys = [
        ("↑↓", "select"),
        ("⏎", "attach"),
        ("c", "cancel"),
        ("r", "rollback"),
        ("R", "reboot"),
        ("d", "deploy main"),
        ("/", "filter"),
        ("?", "help"),
        ("q", "quit"),
    ];
    let hints = keys
        .iter()
        .map(|(key, action)| format!("{} {}", key.bold(), action.dimmed()))
        .collect::<Vec<_>>()
        .join("  ");
    format!(" {hints}")
}

/// a bordered pane titled `title`
fn pane(title: String) -> Block<'static> {
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(Color::DarkGray))
        .title(ansi::line(&format!(" {title} ")))
}

/// styled lines cut to `width` columns, ready for a paragraph
fn text(lines: &[String], width: u16) -> Vec<Line<'static>> {
    lines
        .iter()
        .map(|line| ansi::line(&fit(line, usize::from(width))))
        .collect()
}

/// a dim sentence for a pane with nothing to show
fn quiet(text: impl std::fmt::Display) -> String {
    say::aside(text.to_string().dimmed())
}
