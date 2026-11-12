use std::{fmt, path::PathBuf, str::FromStr};

use eyre::{Context, Report, Result, bail, eyre};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// the fleet's pre-shared NOISE key, written as 64 hex digits
#[derive(Clone, PartialEq, Eq)]
pub struct Key([u8; 32]);

impl Key {
    pub fn generate() -> Self {
        Self(rand::random())
    }

    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    pub fn to_hex(&self) -> String {
        self.0.iter().map(|byte| format!("{byte:02x}")).collect()
    }
}

impl FromStr for Key {
    type Err = Report;

    fn from_str(value: &str) -> Result<Self> {
        let value = value.trim();
        let (pairs, []) = value.as_bytes().as_chunks::<2>() else {
            bail!("a key is exactly 64 hex digits");
        };

        if pairs.len() != 32 {
            bail!("a key is exactly 64 hex digits");
        }

        let mut key = [0u8; 32];
        for (byte, &[high, low]) in key.iter_mut().zip(pairs) {
            *byte = (nibble(high)? << 4) | nibble(low)?;
        }

        Ok(Self(key))
    }
}

fn nibble(digit: u8) -> Result<u8> {
    match digit {
        b'0'..=b'9' => Ok(digit - b'0'),
        b'a'..=b'f' => Ok(digit - b'a' + 10),
        b'A'..=b'F' => Ok(digit - b'A' + 10),
        _ => Err(eyre!("a key contains only hex digits")),
    }
}

/// never prints the key itself
impl fmt::Debug for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Key(..)")
    }
}

impl Serialize for Key {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_hex())
    }
}

impl<'de> Deserialize<'de> for Key {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}

/// where a config finds its key: inline, in a file (an agenix secret), or in
/// `COEUS_KEY` (a CI secret), in that order
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct KeySource {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<Key>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_file: Option<PathBuf>,
}

impl KeySource {
    pub const ENV: &'static str = "COEUS_KEY";

    pub fn load(&self) -> Result<Key> {
        if let Some(key) = &self.key {
            return Ok(key.clone());
        }

        if let Some(path) = &self.key_file {
            return std::fs::read_to_string(path)
                .wrap_err_with(|| format!("failed to read the key from {}", path.display()))?
                .parse();
        }

        match std::env::var(Self::ENV) {
            Ok(value) => value
                .parse()
                .wrap_err_with(|| format!("invalid {}", Self::ENV)),
            Err(_) => Err(eyre!(
                "no key configured: set `key`, `key_file` or {}",
                Self::ENV
            )),
        }
    }
}
