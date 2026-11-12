use std::io::Write;

use eyre::Result;

use super::{Mode, terminal::Passthrough};
use crate::{
    common::model::AttachFrame,
    ui::{journal, say},
};

/// where an attach writes the log
pub(super) enum Sink {
    /// a drawn view was asked for: frames wait until the machine either
    /// draws or sends nix's lines, as a machine that cannot draw does
    Waiting(Vec<AttachFrame>),
    /// the machine's nom draws on this terminal; frames carry only events
    Screen(Passthrough),
    Raw,
    Transcript,
    Silent,
}

impl Sink {
    pub fn open(mode: Mode, drawn: bool) -> Self {
        match mode {
            Mode::Live if drawn => Self::Waiting(Vec::new()),
            Mode::Live | Mode::Transcript => Self::Transcript,
            Mode::Raw => Self::Raw,
            Mode::Silent => Self::Silent,
        }
    }

    /// whether the machine draws, so it takes resizes
    pub const fn draws(&self) -> bool {
        matches!(self, Self::Screen(_))
    }

    pub fn frames(&mut self, frames: &[AttachFrame]) -> Result<()> {
        if let Self::Waiting(waiting) = self {
            if !frames
                .iter()
                .any(|frame| matches!(frame, AttachFrame::Nix(_)))
            {
                waiting.extend_from_slice(frames);
                return Ok(());
            }
            println!(
                "{}",
                say::aside("the machine cannot draw nom's view, so here is its log")
            );
            let waiting = std::mem::take(waiting);
            *self = Self::Transcript;
            self.frames(&waiting)?;
        }

        for frame in frames {
            match (&*self, frame) {
                (Self::Raw, AttachFrame::Nix(bytes)) => {
                    std::io::stdout().lock().write_all(bytes)?
                }
                (Self::Raw, AttachFrame::Event(stamped)) => {
                    if let Some(line) = journal::event(stamped) {
                        eprintln!("{line}");
                    }
                }
                (Self::Transcript, frame) => {
                    for line in journal::frame(frame) {
                        println!("{line}");
                    }
                }
                (Self::Waiting(_) | Self::Screen(_) | Self::Silent, _) => {}
            }
        }
        Ok(())
    }

    /// what the machine's nom drew
    pub fn screen(&mut self, bytes: &[u8]) -> Result<()> {
        if let Self::Waiting(_) = self {
            *self = Self::Screen(Passthrough::enter()?);
        }
        if let Self::Screen(terminal) = self {
            terminal.write(bytes)?;
        }
        Ok(())
    }

    pub fn line(&mut self, text: &str) -> Result<()> {
        match self {
            Self::Screen(terminal) => terminal.write(format!("\r\n{text}\r\n").as_bytes())?,
            Self::Raw => eprintln!("{text}"),
            Self::Waiting(_) | Self::Transcript => println!("{text}"),
            Self::Silent => {}
        }
        Ok(())
    }

    /// shows what still waits and gives the terminal back
    pub fn close(self) -> Result<()> {
        match self {
            Self::Waiting(frames) => Self::Transcript.frames(&frames)?,
            Self::Screen(terminal) => drop(terminal),
            Self::Raw => std::io::stdout().flush()?,
            Self::Transcript | Self::Silent => {}
        }
        Ok(())
    }
}
