use std::{io::Write, process::Command, time::Duration};

use clap::Args;
use colored::Colorize;
use eyre::{Result, bail};

use crate::{
    cli::{Context, context::local_hostname, context::within, exit::Exit},
    client::{Checkup, Severity},
    common::model::{FdaStatus, Hostname},
    ui::{Theme, say},
};

#[derive(Args, Debug)]
pub struct DoctorArgs {
    /// Machines to check (default: every machine of the inventory)
    pub targets: Vec<String>,

    /// On this Mac, walk through granting Full Disk Access to the runner launcher
    #[arg(long)]
    pub fix: bool,

    /// Print the findings as JSON
    #[arg(long)]
    pub json: bool,
}

impl DoctorArgs {
    const SETTINGS: &'static str =
        "x-apple.systempreferences:com.apple.settings.PrivacySecurity.extension?Privacy_AllFiles";
    const PATIENCE: Duration = Duration::from_secs(600);

    pub async fn run(self, context: &Context) -> Result<Exit> {
        if self.fix {
            return Self::fix(context).await;
        }

        let fleet = context.fleet(context.targets_or_all(&self.targets)?)?;
        let checkups: Vec<Checkup> =
            fleet
                .each(|host, mut client| async move {
                    within(client.view()).await.map(|view| (host, view))
                })
                .await
                .into_iter()
                .map(|(host, result)| match result {
                    Ok((_, view)) => Checkup::of(&view, env!("CARGO_PKG_VERSION")),
                    Err(error) => Checkup::unreachable(host, &format!("{error:#}")),
                })
                .collect();

        match self.json {
            true => println!("{}", serde_json::to_string_pretty(&checkups)?),
            false => checkups.iter().for_each(print),
        }

        Ok(match checkups.iter().map(Checkup::severity).max() {
            Some(Severity::Fail) => Exit::Failed,
            _ => Exit::Success,
        })
    }

    /// opens System Settings on the Full Disk Access list with the launcher
    /// revealed beside it, then waits until this Mac's coeusd sees the grant
    async fn fix(context: &Context) -> Result<Exit> {
        if !cfg!(target_os = "macos") {
            bail!("only Macs need Full Disk Access");
        }

        let host: Hostname = local_hostname().parse()?;
        let fleet = context.machine(&host)?;
        let view = within(fleet.connect(&host).await?.view()).await?;

        if view.system.fda == FdaStatus::Granted {
            println!(
                "{}",
                say::success("coeus-launcher already has Full Disk Access")
            );
            return Ok(Exit::Success);
        }

        let launcher = Checkup::LAUNCHER;
        println!(
            "{}",
            say::info("granting Full Disk Access to coeus-launcher")
        );
        println!(
            "  {} drag the {} that Finder shows into the list, then turn it on",
            "1.".bold(),
            "coeus-launcher".bold()
        );
        println!(
            "  {} or press {}, paste {} and press return",
            "  ".bold(),
            "+ then ⌘⇧G".bold(),
            launcher.dimmed()
        );

        let _ = Command::new("open").arg(Self::SETTINGS).status();
        let _ = Command::new("open").args(["-R", launcher]).status();
        if let Ok(mut copy) = Command::new("pbcopy")
            .stdin(std::process::Stdio::piped())
            .spawn()
        {
            if let Some(stdin) = copy.stdin.as_mut() {
                let _ = stdin.write_all(launcher.as_bytes());
            }
            let _ = copy.wait();
        }

        let mut views = fleet.connect(&host).await?.watch().await?;
        let granted = tokio::time::timeout(Self::PATIENCE, async {
            while let Ok(Some(view)) = views.next().await {
                if view.system.fda == FdaStatus::Granted {
                    return true;
                }
            }
            false
        })
        .await
        .unwrap_or(false);

        match granted {
            true => {
                println!(
                    "{}",
                    say::success("coeusd sees the grant: activations can change users now")
                );
                Ok(Exit::Success)
            }
            false => {
                println!(
                    "{}",
                    say::error(
                        "coeusd did not see the grant yet; run `coeus doctor` to check again"
                    )
                );
                Ok(Exit::Failed)
            }
        }
    }
}

fn print(checkup: &Checkup) {
    let glyph = match checkup.severity() {
        Severity::Ok => "✓".green().bold(),
        Severity::Warn => "!".yellow().bold(),
        Severity::Fail => "✗".red().bold(),
    };
    println!("{glyph} {}", Theme::host(checkup.host.as_str()));

    for finding in &checkup.findings {
        let title = match finding.severity {
            Severity::Ok => finding.title.normal(),
            Severity::Warn => finding.title.yellow(),
            Severity::Fail => finding.title.red(),
        };
        println!("    {title}");
        if let Some(fix) = &finding.fix {
            println!("      {} {}", "→".dimmed(), fix.dimmed());
        }
    }
}
