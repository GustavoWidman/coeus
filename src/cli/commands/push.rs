use std::{
    path::Path,
    time::{Duration, SystemTime},
};

use clap::{Args, Subcommand};
use eyre::{Result, eyre};
use gjallar::{Dir, Message, Notification, Topic, Urgency, VapidKey};

use crate::{
    cli::{exit::Exit, local_hostname},
    config::{ClientSettings, Origin},
    push::{Delivery, Push},
    ui::say,
};

#[derive(Args, Debug)]
pub struct PushArgs {
    #[command(subcommand)]
    pub command: PushCommand,
}

#[derive(Subcommand, Debug)]
pub enum PushCommand {
    /// Print a new VAPID private key, its public half on stderr
    Keygen,
    /// Send a test notification to every subscribed device
    Test,
}

impl PushArgs {
    pub async fn run(self, config: Option<&Path>) -> Result<Exit> {
        match self.command {
            PushCommand::Keygen => keygen(),
            PushCommand::Test => test(&ClientSettings::load(config)?).await,
        }
    }
}

/// the private key goes to stdout, so it can go straight into
/// `agenix -e` without landing on the screen
fn keygen() -> Result<Exit> {
    let key = VapidKey::generate()?;
    println!("{}", key.to_base64url());
    eprintln!("public key (applicationServerKey): {}", key.public_key());
    Ok(Exit::Success)
}

/// fails only when the settings or their files can't be used; what the
/// push services answer is printed, never an exit status
async fn test(settings: &ClientSettings) -> Result<Exit> {
    let settings = settings
        .push
        .clone()
        .ok_or_else(|| eyre!("no [settings.push] in the configuration"))?;
    let push = Push::load(&settings)?;
    let message = test_message(&local_hostname(), push.origin())?;

    if push.devices().is_empty() {
        println!(
            "0 subscriptions in {}",
            settings.subscriptions_file.display()
        );
        return Ok(Exit::Success);
    }

    let lines = tokio::task::spawn_blocking(move || {
        push.send(&message)
            .into_iter()
            .map(|(name, delivery)| line(name, &delivery))
            .collect::<Vec<_>>()
    })
    .await?;
    for line in lines {
        println!("{line}");
    }
    Ok(Exit::Success)
}

const TEST_TAG: &str = "coeus/test";

pub fn test_message(host: &str, origin: &Origin) -> Result<Message> {
    let notification = Notification::new(format!("coeus test · from {host}"), origin.join("/"))
        .tag(TEST_TAG)
        .timestamp(SystemTime::now())
        .lang("en")
        .dir(Dir::Ltr);
    Ok(Message::declarative(notification)?
        // a test nobody saw within the hour tests nothing
        .ttl(Duration::from_secs(3600))
        .urgency(Urgency::Normal)
        .topic(Topic::digest(TEST_TAG)))
}

pub fn line(name: &str, delivery: &Delivery) -> String {
    let text = format!("{name}: {delivery}");
    match delivery {
        Delivery::Delivered => say::success(text),
        Delivery::Gone { .. } => say::warn(text),
        Delivery::Refused { .. } => say::error(text),
        Delivery::RetryLater { .. } => say::aside(text),
    }
}
