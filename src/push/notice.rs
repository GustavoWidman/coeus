use std::time::{Duration, SystemTime};

use eyre::Result;
use gjallar::{Dir, Message, Notification, Topic, Urgency};

use crate::config::Origin;

/// a notification as the fleet words it, before it becomes a push message
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notice {
    pub title: String,
    /// at most a few short lines
    pub body: String,
    /// the app's screen a tap opens, a path under the origin
    pub path: String,
    /// a later notice with the same tag replaces this one on the device, and
    /// at the push service while this one waits there undelivered
    pub tag: String,
    /// something broke: worth waking the device for
    pub urgent: bool,
}

impl Notice {
    /// a notice older than a day is noise
    const TTL: Duration = Duration::from_secs(24 * 3600);

    /// the declarative message Safari shows by itself, and the service
    /// worker shows elsewhere
    pub fn message(&self, origin: &Origin, at: SystemTime) -> Result<Message> {
        let mut notification = Notification::new(&self.title, origin.join(&self.path))
            .tag(&self.tag)
            .timestamp(at)
            .lang("en")
            .dir(Dir::Ltr);
        if !self.body.is_empty() {
            notification = notification.body(&self.body);
        }
        let urgency = match self.urgent {
            true => Urgency::High,
            false => Urgency::Normal,
        };
        Ok(Message::declarative(notification)?
            .ttl(Self::TTL)
            .urgency(urgency)
            .topic(Topic::digest(&self.tag)))
    }
}
