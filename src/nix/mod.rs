//! what nix prints, understood: its `internal-json` log and its closure diffs

mod ansi;
mod diff;
mod filter;
mod flake;
mod fold;
mod line;

#[cfg(test)]
mod test;

pub use ansi::strip_ansi;
pub use filter::*;
pub use flake::FlakeRepo;
pub use fold::*;
pub use line::*;
