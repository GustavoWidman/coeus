//! an ssh public key, the one representation of a host key: what ssh pins,
//! what the probe reads, what enrollment writes and what age stanzas name

use std::{fmt, str::FromStr};

use base64::{
    Engine,
    engine::general_purpose::{STANDARD, STANDARD_NO_PAD},
};
use eyre::{Report, bail, eyre};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// `ssh-ed25519 AAAA…`, without the comment, so `root@nixos` and a bare key
/// are the same key
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct SshKey {
    kind: KeyKind,
    /// the key in ssh's wire encoding, what the text form's base64 holds
    blob: Vec<u8>,
}

/// the key types age can encrypt to
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum KeyKind {
    Ed25519,
    Rsa,
}

impl KeyKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ed25519 => "ssh-ed25519",
            Self::Rsa => "ssh-rsa",
        }
    }

    fn parse(name: &str) -> Option<Self> {
        match name {
            "ssh-ed25519" => Some(Self::Ed25519),
            "ssh-rsa" => Some(Self::Rsa),
            _ => None,
        }
    }
}

impl SshKey {
    /// an ed25519 blob: the length-prefixed type name, then the 32-byte key
    const ED25519_BLOB: usize = 4 + "ssh-ed25519".len() + 4 + 32;

    /// `ssh-ed25519 AAAA… [comment]`, as a `.pub` file or `authorized_keys`
    /// line holds it
    pub fn parse(text: &str) -> eyre::Result<Self> {
        let mut words = text.split_whitespace();
        let (Some(name), Some(encoded)) = (words.next(), words.next()) else {
            bail!("not an ssh public key: {text:?}");
        };
        let Some(kind) = KeyKind::parse(name) else {
            bail!("an ssh-ed25519 or ssh-rsa key is needed, not {name}");
        };
        let blob = STANDARD
            .decode(encoded)
            .map_err(|_| eyre!("not an ssh public key, its key is not base64: {text:?}"))?;

        // the blob names its own type; a key whose text and blob disagree was
        // pasted together from two keys
        if blob_kind(&blob) != Some(name.as_bytes()) {
            bail!("not an ssh public key, its key is not a {name} key: {text:?}");
        }
        if kind == KeyKind::Ed25519 && blob.len() != Self::ED25519_BLOB {
            bail!("not an ssh public key, its ed25519 key is cut short: {text:?}");
        }

        Ok(Self { kind, blob })
    }

    pub const fn kind(&self) -> KeyKind {
        self.kind
    }

    /// `SHA256:…`, as `ssh-keygen -l` prints it, to compare by eye
    pub fn fingerprint(&self) -> String {
        format!("SHA256:{}", STANDARD_NO_PAD.encode(self.digest()))
    }

    /// how an age `ssh-ed25519` or `ssh-rsa` stanza names its recipient: the
    /// first four bytes of the blob's SHA-256, in unpadded base64
    pub fn age_tag(&self) -> String {
        STANDARD_NO_PAD.encode(&self.digest()[..4])
    }

    fn digest(&self) -> [u8; 32] {
        Sha256::digest(&self.blob).into()
    }
}

/// the type name a wire-encoded key starts with
fn blob_kind(blob: &[u8]) -> Option<&[u8]> {
    let (length, rest) = blob.split_first_chunk::<4>()?;
    rest.get(..u32::from_be_bytes(*length) as usize)
}

impl fmt::Display for SshKey {
    /// `ssh-ed25519 AAAA…`
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.kind.as_str(), STANDARD.encode(&self.blob))
    }
}

impl fmt::Debug for SshKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SshKey({self})")
    }
}

impl FromStr for SshKey {
    type Err = Report;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Self::parse(text)
    }
}

impl TryFrom<String> for SshKey {
    type Error = Report;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        Self::parse(&text)
    }
}

impl From<SshKey> for String {
    fn from(key: SshKey) -> Self {
        key.to_string()
    }
}
