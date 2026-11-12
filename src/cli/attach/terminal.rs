use std::io::{self, IsTerminal, Write};

use rustix::termios::{LocalModes, OptionalActions, OutputModes, Termios};

use crate::common::proto::TerminalSize;

/// stdout's size, when it is a terminal
pub fn size() -> Option<TerminalSize> {
    let stdout = io::stdout();
    if !stdout.is_terminal() {
        return None;
    }
    let size = rustix::termios::tcgetwinsize(&stdout).ok()?;
    (size.ws_col > 0 && size.ws_row > 0).then_some(TerminalSize {
        columns: size.ws_col,
        rows: size.ws_row,
    })
}

/// the terminal showing another machine's terminal output as it comes, the
/// way `ssh -tt` sets it: output passed through untouched and keys not
/// echoed over it. ctrl-c still interrupts. dropping it restores the terminal
pub struct Passthrough {
    saved: Termios,
    /// whether the output so far ends a line
    ended_line: bool,
}

impl Passthrough {
    /// ends synchronized output and shows the cursor, in case the output
    /// stopped in the middle of a frame
    const RESET: &'static [u8] = b"\x1b[?2026l\x1b[?25h";

    pub fn enter() -> io::Result<Self> {
        let stdout = io::stdout();
        let saved = rustix::termios::tcgetattr(&stdout)?;
        let mut passing = saved.clone();
        passing.local_modes.remove(LocalModes::ECHO);
        passing.output_modes.remove(OutputModes::OPOST);
        rustix::termios::tcsetattr(&stdout, OptionalActions::Now, &passing)?;
        Ok(Self {
            saved,
            ended_line: true,
        })
    }

    pub fn write(&mut self, bytes: &[u8]) -> io::Result<()> {
        let Some(last) = bytes.last() else {
            return Ok(());
        };
        let mut stdout = io::stdout().lock();
        stdout.write_all(bytes)?;
        stdout.flush()?;
        self.ended_line = *last == b'\n';
        Ok(())
    }
}

/// leaves the cursor at the start of a line for what follows
impl Drop for Passthrough {
    fn drop(&mut self) {
        let ended_line = self.ended_line;
        let _ = self.write(Self::RESET);
        if !ended_line {
            let _ = self.write(b"\r\n");
        }
        let _ = rustix::termios::tcsetattr(io::stdout(), OptionalActions::Now, &self.saved);
    }
}
