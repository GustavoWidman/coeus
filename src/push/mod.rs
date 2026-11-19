//! Web Push to the devices in the subscription list, through gjallar

use std::{collections::BTreeSet, fmt, path::Path, time::Duration};

use eyre::{Result, WrapErr, bail, ensure};
use gjallar::{Client, Message, Outcome, Subscription, VapidKey, WebPush};
use serde::Deserialize;
use zeroize::Zeroizing;

use crate::{
    config::{Origin, PushSettings},
    utils::secret,
};

/// how long one push service may take to answer
const TIMEOUT: Duration = Duration::from_secs(5);
/// how long a whole send may take, retries included
const DEADLINE: Duration = Duration::from_secs(10);

/// a sender, holding the VAPID key and the subscriptions as they were when
/// it loaded
pub struct Push {
    client: Client,
    origin: Origin,
    devices: Vec<Device>,
}

/// one entry of the subscription list. the endpoint and its keys are
/// secrets, so only the name is ever printed
#[derive(Debug)]
pub struct Device {
    pub name: String,
    pub subscription: Subscription,
}

/// what a push service said about one device
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Delivery {
    Delivered,
    /// the device dropped the subscription; every later send fails the same
    /// way until its entry leaves the list
    Gone {
        status: u16,
    },
    /// sending it again unchanged gets the same answer
    Refused {
        status: u16,
        reason: Option<String>,
    },
    /// busy, failing or out of reach; the next message may get through
    RetryLater {
        why: String,
    },
}

impl Push {
    /// reads the VAPID key and the subscriptions, opening them with the
    /// identity when there is one
    pub fn load(settings: &PushSettings) -> Result<Self> {
        let identity = match &settings.identity_file {
            Some(path) => Some(
                secret::ssh_identity(&read(path)?)
                    .wrap_err_with(|| format!("in the push identity {}", path.display()))?,
            ),
            None => None,
        };
        let open = |path: &Path| -> Result<Zeroizing<Vec<u8>>> {
            let bytes = read(path)?;
            match &identity {
                Some(identity) => secret::decrypt(identity, &bytes)
                    .wrap_err_with(|| format!("failed to decrypt {}", path.display())),
                None => Ok(bytes),
            }
        };

        let vapid = open(&settings.vapid_key_file)?;
        let key = std::str::from_utf8(&vapid)
            .ok()
            .and_then(|text| VapidKey::from_base64url(text.trim()).ok())
            .ok_or_else(|| {
                eyre::eyre!(
                    "{} does not hold a VAPID private key (the base64url of a P-256 scalar)",
                    settings.vapid_key_file.display()
                )
            })?;
        let devices = devices(&open(&settings.subscriptions_file)?)
            .wrap_err_with(|| format!("in {}", settings.subscriptions_file.display()))?;

        let push = WebPush::new(key, settings.origin.as_str())?;
        Ok(Self {
            client: Client::new(push).timeout(TIMEOUT).deadline(DEADLINE),
            origin: settings.origin.clone(),
            devices,
        })
    }

    pub const fn origin(&self) -> &Origin {
        &self.origin
    }

    pub fn devices(&self) -> &[Device] {
        &self.devices
    }

    /// sends `message` to every device and says how each went, in the
    /// list's order. it blocks for at most ten seconds
    pub fn send(&self, message: &Message) -> Vec<(&str, Delivery)> {
        let subscriptions = self.devices.iter().map(|device| &device.subscription);
        self.devices
            .iter()
            .map(|device| device.name.as_str())
            .zip(self.client.send_all(subscriptions, message))
            .map(|(name, result)| (name, Delivery::of(result)))
            .collect()
    }
}

fn read(path: &Path) -> Result<Zeroizing<Vec<u8>>> {
    std::fs::read(path)
        .map(Zeroizing::new)
        .wrap_err_with(|| format!("failed to read {}", path.display()))
}

/// the subscription list: a JSON array of `PushSubscription.toJSON()`
/// objects, each with a unique `name`
fn devices(json: &[u8]) -> Result<Vec<Device>> {
    #[derive(Deserialize)]
    struct Entry {
        name: String,
        #[serde(flatten)]
        subscription: serde_json::Value,
    }

    let entries: Vec<Entry> = serde_json::from_slice(json)
        .wrap_err("expected a JSON array of subscriptions, each with a name")?;
    let mut names = BTreeSet::new();
    entries
        .into_iter()
        .map(|Entry { name, subscription }| {
            ensure!(!name.trim().is_empty(), "a subscription has an empty name");
            if !names.insert(name.clone()) {
                bail!("two subscriptions are named {name:?}");
            }
            let subscription = serde_json::from_value(subscription)
                .wrap_err_with(|| format!("subscription {name:?}"))?;
            Ok(Device { name, subscription })
        })
        .collect()
}

impl Delivery {
    fn of(result: Result<Outcome, gjallar::Error>) -> Self {
        match result {
            Ok(Outcome::Delivered) => Self::Delivered,
            Ok(Outcome::Gone { status }) => Self::Gone {
                status: status.as_u16(),
            },
            Ok(Outcome::Refused { status, reason }) => Self::Refused {
                status: status.as_u16(),
                reason,
            },
            Ok(Outcome::RetryLater { status, .. }) => Self::RetryLater {
                why: format!("the push service answered {}", status.as_u16()),
            },
            Ok(other) => Self::RetryLater {
                why: format!("{other:?}"),
            },
            Err(error) => Self::RetryLater {
                why: error.to_string(),
            },
        }
    }
}

impl fmt::Display for Delivery {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Delivered => f.write_str("delivered"),
            Self::Gone { status } => write!(
                f,
                "gone ({status}): the device dropped this subscription, remove it from the list"
            ),
            Self::Refused {
                status,
                reason: Some(reason),
            } => write!(f, "refused ({status}): {reason}"),
            Self::Refused {
                status,
                reason: None,
            } => write!(f, "refused ({status})"),
            Self::RetryLater { why } => write!(f, "retry later: {why}"),
        }
    }
}

#[cfg(test)]
mod test;
