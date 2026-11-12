//! the pull request body: the contract's text, then coeus's section between
//! `<!-- coeus:bootstrap -->` markers. a run rewrites only its section, so
//! the person may write anywhere else.

use crate::bootstrap::contract::RepoPath;

pub const START: &str = "<!-- coeus:bootstrap -->";
pub const END: &str = "<!-- /coeus:bootstrap -->";

/// what the section says, plain data the run fills from each step's result
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Evidence {
    pub host: String,
    /// `NixOS 26.05 · x86_64-linux · EFI · 16 GiB`
    pub machine: String,
    /// `ssh-ed25519 AAAA…`
    pub host_key: String,
    pub fingerprint: String,
    /// the admin that enrolled it
    pub enrolled_by: String,
    pub files: Vec<RepoPath>,
    pub created: Vec<RepoPath>,
    pub rekeyed: Vec<RepoPath>,
    /// `job #1 · generation 2 · nixos-system-nova-…`, once switched
    pub switch: Option<String>,
    /// coeusd's version and address, once it answered
    pub coeusd: Option<String>,
    /// how it booted, once known
    pub boot: Option<String>,
}

/// the section, markers included
pub fn section(evidence: &Evidence) -> String {
    todo!()
}

/// `body` with its coeus section replaced by `section`, or appended
pub fn splice(body: &str, section: &str) -> String {
    todo!()
}
