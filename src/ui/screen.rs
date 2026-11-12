use std::io::{Stdout, Write, stdout};

use crossterm::{
    QueueableCommand, cursor,
    terminal::{self, BeginSynchronizedUpdate, Clear, ClearType, EndSynchronizedUpdate},
};
use eyre::Result;

use super::text::width;

/// redraws a block of lines in place below whatever the terminal already
/// shows, the cursor hidden until it is dropped
pub struct Screen {
    out: Stdout,
    /// columns each line on screen took, to know how many rows the block
    /// spans once a narrower terminal rewrapped it
    drawn: Vec<usize>,
}

impl Screen {
    pub fn new() -> Result<Self> {
        let mut out = stdout();
        out.queue(cursor::Hide)?;
        out.flush()?;
        Ok(Self {
            out,
            drawn: Vec::new(),
        })
    }

    /// columns and rows a frame may use. one column stays free: some
    /// terminals wrap a line that fills the last column.
    pub fn size() -> (usize, usize) {
        let (columns, rows) = terminal::size().unwrap_or((80, 24));
        (
            usize::from(columns).saturating_sub(1).max(20),
            usize::from(rows),
        )
    }

    pub fn draw(&mut self, lines: &[String]) -> Result<()> {
        let (columns, _) = terminal::size().unwrap_or((80, 24));
        let columns = usize::from(columns).max(1);
        let rows: usize = self
            .drawn
            .iter()
            .map(|taken| taken.div_ceil(columns).max(1))
            .sum();

        self.out.queue(BeginSynchronizedUpdate)?;
        self.out.queue(cursor::MoveToColumn(0))?;
        if rows > 0 {
            self.out
                .queue(cursor::MoveUp(rows.min(usize::from(u16::MAX)) as u16))?;
        }

        for line in lines {
            self.out.write_all(line.as_bytes())?;
            self.out.queue(Clear(ClearType::UntilNewLine))?;
            self.out.write_all(b"\r\n")?;
        }
        self.out.queue(Clear(ClearType::FromCursorDown))?;
        self.out.queue(EndSynchronizedUpdate)?;
        self.out.flush()?;

        self.drawn = lines.iter().map(|line| width(line)).collect();
        Ok(())
    }
}

impl Drop for Screen {
    fn drop(&mut self) {
        let _ = self.out.queue(cursor::Show);
        let _ = self.out.flush();
    }
}
