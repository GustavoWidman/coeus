//! `/api/push`: what the app needs to turn notifications on for a device,
//! and to tell whether the fleet already sends to it. The subscription list
//! is read on every request, as the senders read it on every send, so an
//! edit reaches the app without a restart. The answer names subscriptions
//! by a hash of their endpoint, so it reveals none of them.

use std::path::PathBuf;

use eyre::Result;
use sha2::{Digest, Sha256};

use super::view::PushView;
use crate::config::WebPushSettings;

/// where `/api/push`'s answer comes from
#[derive(Clone, Debug)]
pub enum Push {
    /// the fleet's: its public key and the list the senders read
    Live(WebPushSettings),
    /// always the same answer
    Fixed(PushView),
}

impl Push {
    pub async fn view(&self) -> Result<PushView> {
        match self {
            Self::Fixed(view) => Ok(view.clone()),
            Self::Live(settings) => {
                let path: PathBuf = settings.subscriptions_file.clone();
                let devices =
                    tokio::task::spawn_blocking(move || crate::push::list(&path)).await??;
                Ok(PushView {
                    key: settings.public_key.clone(),
                    subscribed: devices
                        .iter()
                        .map(|device| endpoint_hash(device.subscription.endpoint()))
                        .collect(),
                    edit: settings.edit.clone(),
                })
            }
        }
    }
}

/// the first 16 hex digits of the SHA-256 of `endpoint`, which the app
/// computes the same way for its own subscription
pub fn endpoint_hash(endpoint: &str) -> String {
    Sha256::digest(endpoint.as_bytes())[..8]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
