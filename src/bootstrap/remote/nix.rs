//! a Mac before nix-darwin's first activation: Determinate Nix installed,
//! and the shell files an installer edited moved out of nix-darwin's way

use eyre::{Result, eyre};

use super::{Call, Stock};
use crate::{
    bootstrap::Step,
    cli::Console,
    ui::checklist::{Live, Mark, Row},
};

/// nix on the machine, as the probe found it
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NixState {
    Installed {
        version: String,
    },
    /// the installer's receipt is there but nix does not answer: an install
    /// a crash cut short
    Broken,
    Missing,
}

/// lines of the installer's output under the row
const TAIL: usize = 3;

/// installs when missing, `repair`s when broken (installing again when that
/// does not do it); each is one root call whose last lines show in the row.
/// returns the version that answers afterwards.
pub async fn converge(stock: &Stock, console: &Console) -> Result<String> {
    let mut state = stock.probed.nix.clone();
    let mut tried = Vec::new();

    loop {
        let call = match state {
            NixState::Installed { version } => return Ok(version),
            NixState::Missing if !tried.contains(&Call::InstallNix) => Call::InstallNix,
            NixState::Broken if !tried.contains(&Call::RepairNix) => Call::RepairNix,
            NixState::Broken if !tried.contains(&Call::InstallNix) => Call::InstallNix,
            NixState::Missing | NixState::Broken => {
                return Err(eyre!(
                    "nix still does not answer on the machine after {}; `/nix/nix-installer \
                 uninstall` there clears a broken install",
                    describe(&tried)
                ));
            }
        };

        let doing = match call {
            Call::RepairNix => "repairing Determinate Nix",
            _ => "installing Determinate Nix",
        };
        let mut tail: Vec<String> = Vec::new();
        let status = stock
            .follow(&call, |line| {
                let line = crate::nix::strip_ansi(line).trim().to_owned();
                if line.is_empty() {
                    return;
                }
                tail.push(line);
                if tail.len() > TAIL {
                    tail.remove(0);
                }
                console.update(Row {
                    step: Step::Nix,
                    mark: Mark::Active,
                    detail: doing.into(),
                    live: vec![Live::Tail(tail.clone())],
                });
            })
            .await?;
        if !status.success() {
            console.note(format!("{doing} failed: {}", tail.join(" · ")));
        }

        tried.push(call);
        state = stock.probe().await?.nix;
    }
}

/// `/etc/bashrc`, `/etc/zshrc` and `/etc/zprofile` moved to
/// `.before-nix-darwin` when nix-darwin did not write them; the ones moved,
/// for a step note
pub async fn move_etc_aside(stock: &Stock) -> Result<Vec<String>> {
    let moved = stock.run(&Call::MoveEtcAside).await?;
    Ok(moved.lines().map(str::to_owned).collect())
}

fn describe(tried: &[Call]) -> String {
    tried
        .iter()
        .map(|call| match call {
            Call::RepairNix => "a repair",
            _ => "an install",
        })
        .collect::<Vec<_>>()
        .join(" and ")
}
