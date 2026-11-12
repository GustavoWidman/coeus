use clap::Args;
use eyre::Result;
use serde_json::json;

use crate::{
    cli::{
        Context,
        exit::Exit,
        follow::{Follow, View},
    },
    client::{FleetEvent, Link},
    common::model::Timestamp,
    ui::{Board, Heading, board::Verdict, status},
};

#[derive(Args, Debug)]
pub struct StatusArgs {
    /// Machines to show [default: @all]
    #[arg(value_name = "TARGETS")]
    pub targets: Vec<String>,

    /// Keep the table up to date until ctrl-c
    #[arg(short, long)]
    pub watch: bool,

    /// Print the machines' views as JSON
    #[arg(long, conflicts_with = "watch")]
    pub json: bool,
}

impl StatusArgs {
    pub async fn run(self, context: &Context) -> Result<Exit> {
        let fleet = context.fleet(context.targets_or_all(&self.targets)?)?;
        let mut board = Board::new(
            Heading::Watch {
                since: Timestamp::now(),
            },
            fleet.hosts(),
        );

        if self.watch {
            let mut follow = Follow::new(board, context.live, View::Status)?;
            follow.run(&fleet, None).await?;
            return Ok(Exit::Success);
        }

        let views = fleet
            .each(|_, mut client| async move { client.view().await })
            .await;

        if self.json {
            let machines: Vec<_> = views
                .iter()
                .map(|(host, view)| match view {
                    Ok(view) => json!({ "host": host, "view": view }),
                    Err(error) => json!({ "host": host, "error": error.to_string() }),
                })
                .collect();
            println!("{}", serde_json::to_string_pretty(&machines)?);
        }

        let mut unreachable = Vec::new();
        for (host, view) in views {
            board.apply(match view {
                Ok(view) => FleetEvent::View {
                    host,
                    view: Box::new(view),
                },
                Err(error) => {
                    unreachable.push(Verdict::Unreachable);
                    FleetEvent::Link {
                        host,
                        link: Link::Unreachable(error.to_string()),
                    }
                }
            });
        }

        if !self.json {
            for line in status::lines(&board, Timestamp::now()) {
                println!("{line}");
            }
        }
        Ok(Exit::of(unreachable, false))
    }
}
