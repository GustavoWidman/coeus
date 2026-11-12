use serde::{Deserialize, Serialize};

/// nix's own counters for one kind of work: builds or downloads
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tally {
    pub done: u32,
    pub expected: u32,
    pub running: u32,
    pub failed: u32,
}

impl Tally {
    pub const fn is_empty(&self) -> bool {
        self.expected == 0 && self.done == 0
    }
}

/// a snapshot of a build's progress, as nix reports it
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Progress {
    pub builds: Tally,
    pub downloads: Tally,
    pub downloaded_bytes: u64,
    pub expected_bytes: u64,
    /// names of the derivations building right now, at most [Progress::ACTIVE]
    pub active: Vec<String>,
}

impl Progress {
    pub const ACTIVE: usize = 4;

    /// share of the work done, or `None` while nix has not said how much
    /// there is (evaluation, or nothing to do)
    pub fn fraction(&self) -> Option<f64> {
        let expected = self.builds.expected + self.downloads.expected;
        if expected == 0 {
            return None;
        }

        let done = self.builds.done + self.downloads.done;
        Some((f64::from(done) / f64::from(expected)).min(1.0))
    }
}
