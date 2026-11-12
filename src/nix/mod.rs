//! what nix prints, understood: its `internal-json` log and its closure diffs

mod ansi;
mod filter;
mod fold;
mod line;

#[cfg(test)]
mod test;

pub use ansi::strip_ansi;
pub use filter::*;
pub use fold::*;
pub use line::*;
