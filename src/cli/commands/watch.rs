use clap::Args;
use eyre::Result;

use crate::{
    cli::{
        Context,
        exit::Exit,
        follow::{Follow, View},
    },
    common::model::Timestamp,
    ui::{Board, Heading},
};

#[derive(Args, Debug)]
pub struct WatchArgs {
    /// Machines to watch [default: @all]
    #[arg(value_name = "TARGETS")]
    pub targets: Vec<String>,
}

impl WatchArgs {
    pub async fn run(self, context: &Context) -> Result<Exit> {
        let fleet = context.fleet(context.targets_or_all(&self.targets)?)?;
        let heading = Heading::Watch {
            since: Timestamp::now(),
        };

        let mut follow = Follow::new(
            Board::new(heading, fleet.hosts()),
            context.live,
            View::Board,
        )?;
        follow.run(&fleet, None).await?;
        Ok(Exit::Success)
    }
}
