use std::io::IsTerminal;

use clap::Parser;
use coeus::{cli::main::MainCLIArgs, config::ClientSettings, ui::say, utils::log::Logger};
use env_logger::Target;

#[tokio::main]
async fn main() {
    let args = MainCLIArgs::parse();
    Logger::init_to(args.level(), Target::Stderr);

    // GitHub Actions renders colors in its logs; other pipes get plain text
    let actions = std::env::var("GITHUB_ACTIONS").as_deref() == Ok("true");
    if !std::io::stdout().is_terminal() && !actions {
        colored::control::set_override(false);
    }

    let config = args.config.clone();
    let exit = args
        .command
        .run(|| ClientSettings::load(config.as_deref()))
        .await;

    match exit {
        Ok(exit) => std::process::exit(exit.code()),
        Err(error) => {
            eprintln!("{}", say::error(format!("{error:#}")));
            std::process::exit(1);
        }
    }
}
