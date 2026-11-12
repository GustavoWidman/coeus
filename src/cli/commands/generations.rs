use clap::Args;
use eyre::Result;

use crate::{
    cli::{Context, exit::Exit},
    common::model::{Hostname, Timestamp},
    ui::history,
};

#[derive(Args, Debug)]
pub struct GenerationsArgs {
    pub host: Hostname,

    /// Print the generations as JSON
    #[arg(long)]
    pub json: bool,
}

impl GenerationsArgs {
    pub async fn run(self, context: &Context) -> Result<Exit> {
        let fleet = context.machine(&self.host)?;
        let generations = fleet.connect(&self.host).await?.generations().await?;

        match self.json {
            true => println!("{}", serde_json::to_string_pretty(&generations)?),
            false => {
                for line in history::generations(&generations, Timestamp::now()) {
                    println!("{line}");
                }
            }
        }
        Ok(Exit::Success)
    }
}
