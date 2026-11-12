use serde::{Deserialize, Serialize};

/// what a new system changes against the running one, per package
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClosureDiff {
    pub changes: Vec<PackageChange>,
}

impl ClosureDiff {
    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }

    /// net closure growth in bytes
    pub fn size_delta(&self) -> i64 {
        self.changes.iter().map(|change| change.size_delta).sum()
    }
}

/// one line of `nix store diff-closures`, e.g. `firefox: 120.0 → 121.0, +1.2 MiB`
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageChange {
    pub name: String,
    /// versions in the running system, empty when the package is new
    pub before: Vec<String>,
    /// versions in the new system, empty when the package is gone
    pub after: Vec<String>,
    pub size_delta: i64,
}

impl PackageChange {
    pub fn is_added(&self) -> bool {
        self.before.is_empty() && !self.after.is_empty()
    }

    pub fn is_removed(&self) -> bool {
        self.after.is_empty() && !self.before.is_empty()
    }
}
