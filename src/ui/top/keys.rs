use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use super::{Intent, Request, Top};
use crate::{
    common::model::{Hostname, MachineView, Timestamp},
    ui::{Theme, say},
};

/// what the keyboard currently drives
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Mode {
    Browse,
    /// typing into the filter, which applies as it is typed
    Filter,
    /// a request waits for a yes
    Confirm(Request),
    Help,
}

/// what a key asks of the world outside the dashboard
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    Quit,
    /// follow the machine's current job through nom, then come back
    Attach(Hostname),
    Run(Request),
}

impl Top {
    const PAGE: isize = 10;

    pub fn key(&mut self, key: KeyEvent, now: Timestamp) -> Option<Effect> {
        if key.kind == KeyEventKind::Release {
            return None;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            return Some(Effect::Quit);
        }
        self.touch();

        match std::mem::replace(&mut self.mode, Mode::Browse) {
            Mode::Browse => self.browse(key, now),
            Mode::Filter => {
                self.filter_key(key);
                None
            }
            Mode::Confirm(request) => match key.code {
                KeyCode::Char('y' | 'Y') | KeyCode::Enter => Some(Effect::Run(request)),
                KeyCode::Char('n' | 'N' | 'q') | KeyCode::Esc => None,
                _ => {
                    self.mode = Mode::Confirm(request);
                    None
                }
            },
            Mode::Help => None,
        }
    }

    fn browse(&mut self, key: KeyEvent, now: Timestamp) -> Option<Effect> {
        match key.code {
            KeyCode::Char('q') => return Some(Effect::Quit),
            KeyCode::Esc if self.filter.is_empty() => return Some(Effect::Quit),
            KeyCode::Esc => self.filter.clear(),
            KeyCode::Up | KeyCode::Char('k') => self.step(-1),
            KeyCode::Down | KeyCode::Char('j') => self.step(1),
            KeyCode::PageUp => self.step(-Self::PAGE),
            KeyCode::PageDown => self.step(Self::PAGE),
            KeyCode::Home | KeyCode::Char('g') => self.select_at(0),
            KeyCode::End | KeyCode::Char('G') => self.step(isize::MAX),
            KeyCode::Char('/') => self.mode = Mode::Filter,
            KeyCode::Char('?') => self.mode = Mode::Help,
            KeyCode::Enter | KeyCode::Char('a') => return self.attach(now),
            KeyCode::Char('c') => self.ask(now, |view| match (&view.running, &view.next) {
                (Some(running), _) => Ok(Intent::Cancel {
                    job: running.summary.id,
                }),
                (None, Some(next)) => Ok(Intent::Cancel { job: next.id }),
                (None, None) => Err("has nothing to cancel"),
            }),
            KeyCode::Char('r') => self.ask(now, |view| {
                Ok(Intent::Rollback {
                    from: view.system.generation,
                })
            }),
            KeyCode::Char('R') => self.ask(now, |_| Ok(Intent::Reboot)),
            KeyCode::Char('d') => self.ask(now, |_| Ok(Intent::Deploy)),
            _ => {}
        }
        None
    }

    fn filter_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Enter => return,
            KeyCode::Esc => {
                self.filter.clear();
                return;
            }
            KeyCode::Backspace => {
                self.filter.pop();
            }
            KeyCode::Up => self.step(-1),
            KeyCode::Down => self.step(1),
            KeyCode::Char(char) => self.filter.push(char),
            _ => {}
        }
        self.mode = Mode::Filter;
    }

    fn attach(&mut self, now: Timestamp) -> Option<Effect> {
        let row = self.selected()?;
        let host = row.host.clone();
        match &row.view {
            Some(view) if view.latest().is_some() => Some(Effect::Attach(host)),
            Some(_) => {
                self.refuse(&host, "has no job to attach to", now);
                None
            }
            None => {
                self.refuse(&host, "is not connected", now);
                None
            }
        }
    }

    /// asks to confirm what `intent` makes of the selected machine's view
    fn ask(
        &mut self,
        now: Timestamp,
        intent: impl FnOnce(&MachineView) -> Result<Intent, &'static str>,
    ) {
        let Some(row) = self.selected() else {
            return;
        };
        let host = row.host.clone();
        let Some(view) = &row.view else {
            self.refuse(&host, "is not connected", now);
            return;
        };

        match intent(view) {
            Ok(intent) => self.mode = Mode::Confirm(Request { host, intent }),
            Err(reason) => self.refuse(&host, reason, now),
        }
    }

    fn refuse(&mut self, host: &Hostname, reason: &str, now: Timestamp) {
        self.notify(
            say::aside(format!("{} {reason}", Theme::host(host.as_str()))),
            now,
        );
    }
}
