use std::{fmt, str::FromStr};

use eyre::{Report, bail, eyre};
use serde::{Deserialize, Serialize};

/// a deployment on one machine, numbered by that machine in order of arrival
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct JobId(pub u64);

impl JobId {
    pub const fn next(self) -> Self {
        Self(self.0 + 1)
    }
}

impl fmt::Display for JobId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{}", self.0)
    }
}

impl FromStr for JobId {
    type Err = Report;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let digits = value.strip_prefix('#').unwrap_or(value);
        Ok(Self(digits.parse().map_err(|_| eyre!("invalid job id: {value}"))?))
    }
}

/// one `coeus deploy` invocation, shared by every machine it reaches. also the
/// idempotency key of an order: a machine answers a repeated order with the job
/// it already created for it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct OrderId(pub u64);

impl OrderId {
    pub fn random() -> Self {
        Self(rand::random())
    }
}

impl fmt::Display for OrderId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:016x}", self.0)
    }
}

/// a machine's name, as its daemon reports it and as the inventory lists it
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Hostname(String);

impl Hostname {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for Hostname {
    type Error = Report;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        let valid = !value.is_empty()
            && value.len() <= 253
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_'));

        if !valid {
            bail!("invalid hostname: {value:?}");
        }

        Ok(Self(value))
    }
}

impl FromStr for Hostname {
    type Err = Report;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::try_from(value.to_owned())
    }
}

impl From<Hostname> for String {
    fn from(value: Hostname) -> Self {
        value.0
    }
}

impl fmt::Display for Hostname {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// a full git commit hash
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct CommitSha([u8; 20]);

impl CommitSha {
    pub fn short(&self) -> String {
        self.to_string()[..7].to_owned()
    }
}

impl FromStr for CommitSha {
    type Err = Report;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let (pairs, []) = value.as_bytes().as_chunks::<2>() else {
            bail!("invalid commit hash: {value}");
        };

        if pairs.len() != 20 {
            bail!("a commit hash has 40 hex digits: {value}");
        }

        let mut sha = [0u8; 20];
        for (byte, pair) in sha.iter_mut().zip(pairs) {
            let pair = std::str::from_utf8(pair)?;
            *byte = u8::from_str_radix(pair, 16).map_err(|_| eyre!("invalid commit hash: {value}"))?;
        }

        Ok(Self(sha))
    }
}

impl fmt::Display for CommitSha {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.iter().try_for_each(|byte| write!(f, "{byte:02x}"))
    }
}

impl fmt::Debug for CommitSha {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "CommitSha({self})")
    }
}

impl Serialize for CommitSha {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for CommitSha {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}

/// an absolute path inside the nix store
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct StorePath(String);

impl StorePath {
    const PREFIX: &'static str = "/nix/store/";
    const HASH_LEN: usize = 32;

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// the part after the hash, e.g. `nixos-system-lab-26.11.20261110.a1b2c3d`
    pub fn name(&self) -> &str {
        &self.0[Self::PREFIX.len() + Self::HASH_LEN + 1..]
    }
}

impl TryFrom<String> for StorePath {
    type Error = Report;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        let Some(rest) = value.strip_prefix(Self::PREFIX) else {
            bail!("not a store path: {value}");
        };

        if rest.len() <= Self::HASH_LEN + 1 || rest.as_bytes()[Self::HASH_LEN] != b'-' {
            bail!("not a store path: {value}");
        }

        Ok(Self(value))
    }
}

impl FromStr for StorePath {
    type Err = Report;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::try_from(value.trim_end_matches('/').to_owned())
    }
}

impl From<StorePath> for String {
    fn from(value: StorePath) -> Self {
        value.0
    }
}

impl fmt::Display for StorePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// the SRI hash of a NAR serialization, e.g. `sha256-Qk3…=`
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct NarHash(String);

impl NarHash {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// the hash as a single path component (base64 may contain `/`)
    pub fn file_name(&self) -> String {
        self.0.replace('/', "_").replace('+', "-")
    }
}

impl TryFrom<String> for NarHash {
    type Error = Report;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        let valid = value.strip_prefix("sha256-").is_some_and(|digest| {
            digest.len() == 44
                && digest
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/' | b'='))
        });

        if !valid {
            bail!("not a sha256 SRI hash: {value}");
        }

        Ok(Self(value))
    }
}

impl From<NarHash> for String {
    fn from(value: NarHash) -> Self {
        value.0
    }
}

impl fmt::Display for NarHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
