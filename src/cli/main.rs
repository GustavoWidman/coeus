use std::path::PathBuf;

use clap::{ArgAction, Parser, Subcommand};
use log::LevelFilter;

use super::commands::{
    AttachArgs, BootstrapArgs, CancelArgs, CompletionsArgs, DeployArgs, DoctorArgs,
    GenerationsArgs, HistoryArgs, LogsArgs, RebootArgs, RollbackArgs, StatusArgs, TopArgs,
    WatchArgs,
};

#[derive(Parser, Debug)]
#[command(
    name = "coeus",
    version,
    about = "Deploy NixOS and nix-darwin machines through their coeusd",
    after_help = "Targets are machine names, @all, @<tag> from the inventory, and !name or !@tag exclusions."
)]
pub struct MainCLIArgs {
    /// Path to the configuration file
    #[arg(short, long, global = true, value_name = "FILE", env = "COEUS_CONFIG")]
    pub config: Option<PathBuf>,

    /// Log more; repeat for even more
    #[arg(short, long, global = true, action = ArgAction::Count)]
    pub verbose: u8,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Order a deployment and follow it
    Deploy(DeployArgs),
    /// Show what every machine runs and does
    Status(StatusArgs),
    /// Follow everything happening on the fleet, whoever ordered it
    Watch(WatchArgs),
    /// Watch and steer the fleet on a full-screen dashboard
    Top(TopArgs),
    /// Follow a job's build through nix-output-monitor
    Attach(AttachArgs),
    /// Print a job's log as plain text
    Logs(LogsArgs),
    /// List past jobs
    History(HistoryArgs),
    /// Cancel queued or running jobs before they activate
    Cancel(CancelArgs),
    /// Switch a machine back to an earlier generation
    Rollback(RollbackArgs),
    /// List a machine's system generations
    Generations(GenerationsArgs),
    /// Reboot a machine
    Reboot(RebootArgs),
    /// Bring a machine that runs NixOS, or a Mac, into the fleet
    Bootstrap(BootstrapArgs),
    /// Check machines for problems and say how to fix them
    Doctor(DoctorArgs),
    /// Generate a new fleet key
    Keygen,
    /// Print shell completions
    Completions(CompletionsArgs),
}

impl MainCLIArgs {
    pub const fn level(&self) -> LevelFilter {
        match self.verbose {
            0 => LevelFilter::Warn,
            1 => LevelFilter::Info,
            2 => LevelFilter::Debug,
            _ => LevelFilter::Trace,
        }
    }
}
