use clap::Args;
use eyre::{Result, eyre};

use crate::{
    cli::{
        Context,
        deployment::{Deployment, Following},
        dispatch::Shipment,
        exit::Exit,
        rollout::Rollout,
    },
    client::{Fleet, detect_origin},
    common::model::{Action, Generation, Hostname, NixOptions, Order, OrderId, Source, Timestamp},
    ui::{Theme, format, say},
};

#[derive(Args, Debug)]
pub struct RollbackArgs {
    pub host: Hostname,

    /// The generation to switch to [default: the one before the running one]
    #[arg(long, value_name = "N")]
    pub to: Option<u32>,

    /// Print the machine's answer and exit; the job runs on its own
    #[arg(short, long)]
    pub detach: bool,

    /// Show a desktop notification when done
    #[arg(long)]
    pub notify: bool,
}

impl RollbackArgs {
    pub async fn run(self, context: &Context) -> Result<Exit> {
        let fleet = context.machine(&self.host)?;
        let generation = match self.to {
            Some(number) => number,
            None => self.previous(&fleet).await?,
        };

        println!(
            "{}",
            say::info(format!(
                "rolling {} back to generation {}",
                Theme::host(self.host.as_str()),
                generation
            ))
        );

        let following = match () {
            _ if self.detach => Following::Detach,
            _ if context.live => Following::Attach,
            _ => Following::Board { live: false },
        };

        Deployment {
            rollout: Rollout::plan(fleet.hosts(), &[], None, 0, false)?,
            fleet,
            shipment: Shipment::order(Order {
                id: OrderId::random(),
                source: Source::Generation(generation),
                action: Action::Switch,
                options: NixOptions::default(),
                origin: detect_origin(),
                now: false,
            }),
            following,
            notify: self.notify,
            skip_unreachable: false,
        }
        .run()
        .await
    }

    /// the newest generation older than the running one
    async fn previous(&self, fleet: &Fleet) -> Result<u32> {
        let generations = fleet.connect(&self.host).await?.generations().await?;
        let current = generations
            .iter()
            .find(|generation| generation.current)
            .ok_or_else(|| eyre!("{} does not report which generation runs", self.host))?;

        let previous: &Generation = generations
            .iter()
            .filter(|generation| generation.number < current.number)
            .max_by_key(|generation| generation.number)
            .ok_or_else(|| eyre!("{} has no generation before {}", self.host, current.number))?;

        println!(
            "{}",
            say::aside(format!(
                "generation {} is from {}",
                previous.number,
                format::ago(previous.created_at, Timestamp::now())
            ))
        );
        Ok(previous.number)
    }
}
