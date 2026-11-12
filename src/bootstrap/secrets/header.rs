//! who an age file is encrypted to, read without any key:
//!
//! ```text
//! age-encryption.org/v1
//! -> ssh-ed25519 <tag> <share>      one stanza per recipient
//! <body>
//! -> X25519 <share>
//! <body>
//! -> 8d-grease …                    random stanzas age adds; not recipients
//! --- <mac>
//! <payload>
//! ```
//!
//! an armored file is the same behind `-----BEGIN AGE ENCRYPTED FILE-----`.

use std::path::Path;

use crate::bootstrap::SshKey;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Header {
    pub stanzas: Vec<Stanza>,
    pub armored: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stanza {
    /// `ssh-ed25519` and `ssh-rsa` name their recipient by
    /// [SshKey::age_tag]
    Ssh { tag: String },
    /// X25519 names nobody: only how many there are can be compared
    X25519,
    /// grease, and stanza types coeus does not know
    Other,
}

/// how a file stands against its recipients
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// the ssh tags equal the recipients' as sets, and no X25519 stanza
    /// is left over
    Current,
    /// `missing` recipients must be added, `extra` stanzas dropped
    Stale { missing: usize, extra: usize },
    /// not an age file coeus can read the header of
    Unreadable(String),
}

impl Header {
    /// reads up to the `---` line only; the payload is never read
    pub fn read(path: &Path) -> eyre::Result<Self> {
        // TODO: armored → strip the BEGIN/END lines and base64-decode just
        // enough lines to reach `---`; binary → read lines up to `---`.
        // stanza lines are `-> <type> <args…>`, each followed by body lines
        todo!()
    }

    /// against the recipients the contract gives. duplicates on either side
    /// collapse, since `[ host ] ++ admins` may name a key twice
    pub fn verdict(&self, recipients: &[SshKey]) -> Verdict {
        todo!()
    }

    /// whether `key` is among the recipients, to check an admin before any
    /// sudo prompt
    pub fn names(&self, key: &SshKey) -> bool {
        todo!()
    }
}
