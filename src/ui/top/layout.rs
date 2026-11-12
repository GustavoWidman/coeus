use ratatui::layout::{Constraint, Layout, Rect};

/// where each pane goes on a screen of a given size
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Panes {
    pub header: Rect,
    pub fleet: Rect,
    pub detail: Rect,
    pub log: Rect,
    pub footer: Rect,
    pub arrangement: Arrangement,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Arrangement {
    /// the fleet and the selected machine side by side, the log below
    Wide,
    /// every pane stacked
    Narrow,
}

impl Arrangement {
    /// below this many columns, side by side panes get too cramped to read
    pub const WIDE: u16 = 100;

    pub const fn of(width: u16) -> Self {
        match width >= Self::WIDE {
            true => Self::Wide,
            false => Self::Narrow,
        }
    }
}

impl Panes {
    /// a pane's border takes a row above and below
    const BORDERS: u16 = 2;

    /// `machines` sizes the fleet pane when it is stacked
    pub fn of(area: Rect, machines: usize) -> Self {
        let [header, body, footer] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Fill(1),
            Constraint::Length(1),
        ])
        .areas(area);

        let arrangement = Arrangement::of(area.width);
        let (fleet, detail, log) = match arrangement {
            Arrangement::Wide => Self::wide(body),
            Arrangement::Narrow => Self::narrow(body, machines),
        };

        Self {
            header,
            fleet,
            detail,
            log,
            footer,
            arrangement,
        }
    }

    fn wide(body: Rect) -> (Rect, Rect, Rect) {
        let log = (body.height * 3 / 10).max(6);
        let [top, log] =
            Layout::vertical([Constraint::Min(8), Constraint::Length(log)]).areas(body);

        let fleet = (body.width * 46 / 100).clamp(52, 84);
        let [fleet, detail] =
            Layout::horizontal([Constraint::Length(fleet), Constraint::Fill(1)]).areas(top);
        (fleet, detail, log)
    }

    fn narrow(body: Rect, machines: usize) -> (Rect, Rect, Rect) {
        let rows = u16::try_from(machines).unwrap_or(u16::MAX);
        let fleet = rows
            .saturating_add(Self::BORDERS)
            .min(body.height / 3)
            .max(Self::BORDERS + 1);
        let log = (body.height / 4).max(5);

        let [fleet, detail, log] = Layout::vertical([
            Constraint::Length(fleet),
            Constraint::Min(6),
            Constraint::Length(log),
        ])
        .areas(body);
        (fleet, detail, log)
    }
}
