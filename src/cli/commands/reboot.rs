use std::io::{BufRead, IsTerminal, Write};

use clap::Args;
use eyre::{Result, bail};

use crate::{
    cli::{Context, context::within, exit::Exit},
    common::model::Hostname,
    ui::{Theme, say},
};

#[derive(Args, Debug)]
pub struct RebootArgs {
    pub host: Hostname,

    /// Reboot without asking
    #[arg(short, long)]
    pub yes: bool,
}

impl RebootArgs {
    pub async fn run(self, context: &Context) -> Result<Exit> {
        let host = Theme::host(self.host.as_str());
        if !self.yes && !confirm(&format!("reboot {host} now?"))? {
            println!("{}", say::aside("not rebooting"));
            return Ok(Exit::Success);
        }

        let fleet = context.machine(&self.host)?;
        within(fleet.connect(&self.host).await?.reboot()).await?;
        println!("{}", say::success(format!("{host} is rebooting")));
        Ok(Exit::Success)
    }
}

/// a yes or no from the person at the terminal
fn confirm(question: &str) -> Result<bool> {
    if !std::io::stdin().is_terminal() {
        bail!("refusing to reboot without a terminal to confirm on; pass --yes");
    }

    eprint!("{} [y/N] ", say::warn(question));
    std::io::stderr().flush()?;
    let mut answer = String::new();
    std::io::stdin().lock().read_line(&mut answer)?;
    Ok(matches!(answer.trim(), "y" | "Y" | "yes"))
}
