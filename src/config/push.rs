use std::{fmt, path::PathBuf, str::FromStr};

use eyre::{Report, Result, bail};
use serde::{Deserialize, Serialize};

/// where pushes come from and who receives them. both files are read at
/// every send, so a deploy that changes them needs no restart
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushSettings {
    /// the dashboard's origin: the VAPID subject, and where a tap leads
    pub origin: Origin,
    /// the base64url of a P-256 private scalar (`coeus push keygen`)
    pub vapid_key_file: PathBuf,
    /// a JSON array of `PushSubscription.toJSON()` objects, each with a `name`
    pub subscriptions_file: PathBuf,
    /// an ssh private key that opens both files, when they are still
    /// age-encrypted
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity_file: Option<PathBuf>,
}

/// an `https` origin, no path: `https://coeus.example.com`
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Origin(String);

impl Origin {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// `path` under this origin
    pub fn join(&self, path: &str) -> String {
        format!("{}/{}", self.0, path.trim_start_matches('/'))
    }
}

impl FromStr for Origin {
    type Err = Report;

    fn from_str(value: &str) -> Result<Self> {
        let Some(authority) = value.strip_prefix("https://") else {
            bail!("an origin starts with https:// ({value:?})");
        };
        let authority = authority.strip_suffix('/').unwrap_or(authority);
        if authority.is_empty()
            || authority
                .chars()
                .any(|c| c.is_whitespace() || matches!(c, '/' | '?' | '#' | '@'))
        {
            bail!("an origin is https:// and a host, with no path ({value:?})");
        }
        Ok(Self(format!("https://{authority}")))
    }
}

impl TryFrom<String> for Origin {
    type Error = Report;

    fn try_from(value: String) -> Result<Self> {
        value.parse()
    }
}

impl From<Origin> for String {
    fn from(origin: Origin) -> Self {
        origin.0
    }
}

impl fmt::Debug for Origin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.0, f)
    }
}

impl fmt::Display for Origin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
