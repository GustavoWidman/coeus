use std::{
    io::{self, Stdout, stdout},
    sync::Once,
};

use crossterm::{
    cursor, execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use eyre::Result;
use ratatui::{Frame, Terminal, backend::CrosstermBackend};

/// the alternate screen in raw mode, given back on drop, on panic, and for
/// as long as something else needs the terminal
pub struct Fullscreen {
    terminal: Terminal<CrosstermBackend<Stdout>>,
}

impl Fullscreen {
    pub fn enter() -> Result<Self> {
        static HOOK: Once = Once::new();
        HOOK.call_once(|| {
            let previous = std::panic::take_hook();
            std::panic::set_hook(Box::new(move |info| {
                let _ = leave();
                previous(info);
            }));
        });

        enter()?;
        Ok(Self {
            terminal: Terminal::new(CrosstermBackend::new(stdout()))?,
        })
    }

    pub fn draw(&mut self, render: impl FnOnce(&mut Frame<'_>)) -> Result<()> {
        self.terminal.draw(render)?;
        Ok(())
    }

    /// the normal screen in cooked mode, e.g. for nom
    pub fn suspend(&mut self) -> Result<()> {
        leave()?;
        Ok(())
    }

    pub fn resume(&mut self) -> Result<()> {
        enter()?;
        self.terminal.clear()?;
        Ok(())
    }
}

impl Drop for Fullscreen {
    fn drop(&mut self) {
        let _ = leave();
    }
}

fn enter() -> io::Result<()> {
    enable_raw_mode()?;
    execute!(stdout(), EnterAlternateScreen, cursor::Hide)
}

fn leave() -> io::Result<()> {
    disable_raw_mode()?;
    execute!(stdout(), LeaveAlternateScreen, cursor::Show)
}
