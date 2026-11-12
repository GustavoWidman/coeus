mod attach;
mod cancel;
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

pub use attach::{AttachArgs, LogsArgs};
pub use cancel::CancelArgs;
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

use eyre::Result;

use super::{Context, exit::Exit, main::Command};
use crate::config::ClientSettings;

impl Command {
    pub async fn run(self, settings: impl FnOnce() -> Result<ClientSettings>) -> Result<Exit> {
        let context = || settings().map(Context::new);

        match self {
            Self::Deploy(args) => args.run(&context()?).await,
            Self::Status(args) => args.run(&context()?).await,
            Self::Watch(args) => args.run(&context()?).await,
            Self::Attach(args) => args.run(&context()?).await,
            Self::Logs(args) => args.run(&context()?).await,
            Self::History(args) => args.run(&context()?).await,
            Self::Cancel(args) => args.run(&context()?).await,
            Self::Rollback(args) => args.run(&context()?).await,
            Self::Generations(args) => args.run(&context()?).await,
            Self::Reboot(args) => args.run(&context()?).await,
            Self::Doctor(args) => args.run(&context()?).await,
            Self::Top(args) => args.run(&context()?).await,
            Self::Keygen => Ok(keygen()),
            Self::Completions(args) => Ok(args.run()),
        }
    }
}
