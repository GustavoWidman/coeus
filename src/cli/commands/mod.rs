mod attach;
mod bootstrap;
mod cancel;
mod catch_up;
mod deploy;
mod doctor;
mod generations;
mod history;
mod reboot;
pub(super) mod rollback;
mod setup;
mod status;
mod top;
mod watch;
mod web;

pub use attach::{AttachArgs, LogsArgs};
pub use bootstrap::BootstrapArgs;
pub use cancel::CancelArgs;
pub use catch_up::CatchUpArgs;
pub use deploy::DeployArgs;
pub use doctor::DoctorArgs;
pub use generations::GenerationsArgs;
pub use history::HistoryArgs;
pub use reboot::RebootArgs;
pub use rollback::RollbackArgs;
pub use setup::{CompletionsArgs, keygen};
pub use status::StatusArgs;
pub use top::TopArgs;
pub use watch::WatchArgs;
pub use web::WebArgs;

use std::path::Path;

use eyre::Result;

use super::{Context, exit::Exit, main::Command};
use crate::config::ClientSettings;

impl Command {
    /// runs the command with the settings at `config`, or at the command's
    /// default location
    pub async fn run(self, config: Option<&Path>) -> Result<Exit> {
        let context = || ClientSettings::load(config).map(Context::new);

        match self {
            Self::Deploy(args) => args.run(&context()?).await,
            Self::Status(args) => args.run(&context()?).await,
            Self::Watch(args) => args.run(&context()?).await,
            Self::Attach(args) => args.run(&context()?).await,
            Self::Logs(args) => args.run(&context()?).await,
            Self::History(args) => args.run(&context()?).await,
            Self::Cancel(args) => args.run(&context()?).await,
            Self::CatchUp(args) => args.run(&context()?).await,
            Self::Rollback(args) => args.run(&context()?).await,
            Self::Generations(args) => args.run(&context()?).await,
            Self::Reboot(args) => args.run(&context()?).await,
            Self::Bootstrap(args) => args.run(&context()?).await,
            Self::Doctor(args) => args.run(&context()?).await,
            Self::Top(args) => args.run(&context()?).await,
            Self::Web(args) => args.run(config).await,
            Self::Keygen => Ok(keygen()),
            Self::Completions(args) => Ok(args.run()),
        }
    }
}
