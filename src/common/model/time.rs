use std::{fmt, time::Duration};

use chrono::{DateTime, Local, Utc};
use serde::{Deserialize, Serialize};

/// milliseconds since the unix epoch: compact on the wire and in job logs
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Timestamp(pub i64);

impl Timestamp {
    pub fn now() -> Self {
        Self(Utc::now().timestamp_millis())
    }

    /// time from `self` until `later`, zero if `later` is earlier
    pub fn until(self, later: Self) -> Duration {
        Duration::from_millis(later.0.saturating_sub(self.0).max(0) as u64)
    }

    pub fn elapsed(self) -> Duration {
        self.until(Self::now())
    }

    pub fn to_local(self) -> DateTime<Local> {
        DateTime::<Utc>::from_timestamp_millis(self.0)
            .unwrap_or_default()
            .with_timezone(&Local)
    }
}

impl fmt::Display for Timestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_local().format("%Y-%m-%d %H:%M:%S"))
    }
}
