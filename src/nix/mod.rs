//! what nix prints, understood: its `internal-json` log and its closure diffs

mod ansi;
mod diff;
mod filter;
mod flake;
mod fold;
mod line;

#[cfg(test)]
mod test;

/// the features every nix call reading a fleet's flake enables: a fleet may
/// write `|>` anywhere, and a stock machine's nix has it off
pub const FLAKE_FEATURES: &str = "nix-command flakes pipe-operators";

pub use ansi::strip_ansi;
pub use filter::*;
pub use flake::FlakeRepo;
pub use fold::*;
pub use line::*;
