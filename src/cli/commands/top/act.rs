use eyre::Result;

use super::super::{cancel, rollback};
use crate::{
    cli::{context::within, deployment::placement},
    client::{CoeusClient, Fleet, detect_origin},
    common::model::{Action, Hostname, JobSelector, NixOptions, Order, OrderId, Source},
    ui::{
        Theme, say,
        top::{Intent, Request},
    },
};

/// carries out a confirmed request and says how the machine answered
pub async fn perform(fleet: Fleet, request: Request) -> String {
    let host = &request.host;
    let answered = async {
        let client = fleet.connect(host).await?;
        answer(client, host, &request.intent).await
    };

    answered
        .await
        .unwrap_or_else(|error| say::error(format!("{}: {error:#}", Theme::host(host.as_str()))))
}

async fn answer(mut client: CoeusClient, host: &Hostname, intent: &Intent) -> Result<String> {
    match intent {
        Intent::Cancel { job } => {
            let report = within(client.cancel(JobSelector::Id(*job))).await?;
            Ok(cancel::answer(host, report))
        }
        Intent::Reboot => {
            within(client.reboot()).await?;
            Ok(say::success(format!(
                "{} is rebooting",
                Theme::host(host.as_str())
            )))
        }
        Intent::Rollback { .. } => {
            let generations = within(client.generations()).await?;
            let previous = rollback::previous(host, &generations)?;
            order(client, host, Source::Generation(previous.number)).await
        }
        Intent::Deploy => {
            let pinned = within(client.resolve(Request::DEPLOY_REF)).await?;
            order(client, host, pinned.into()).await
        }
    }
}

/// a switch to `source`, ordered the way `coeus deploy` orders it
async fn order(mut client: CoeusClient, host: &Hostname, source: Source) -> Result<String> {
    let order = Order {
        id: OrderId::random(),
        source,
        action: Action::Switch,
        options: NixOptions::default(),
        origin: detect_origin(),
        now: false,
    };
    let answer = within(client.deploy(order)).await?;
    Ok(placement(host, &Ok(answer)))
}
