use std::io::IsTerminal;

use clap::{Args, CommandFactory, ValueEnum};
use clap_complete::{Generator, Shell as Builtin};
use clap_complete_nushell::Nushell;

use crate::{
    cli::{exit::Exit, main::MainCLIArgs},
    config::Key,
    ui::say,
};

/// prints a fresh fleet key; the hint goes to stderr so `coeus keygen > key`
/// writes only the key
pub fn keygen() -> Exit {
    println!("{}", Key::generate().to_hex());
    if std::io::stdout().is_terminal() {
        eprintln!(
            "{}",
            say::aside(
                "set it as `key` (or point `key_file` at it) in coeus.toml and in every coeusd.toml"
            )
        );
    }
    Exit::Success
}

#[derive(Args, Debug)]
pub struct CompletionsArgs {
    pub shell: Shell,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum Shell {
    Bash,
    Zsh,
    Fish,
    Nushell,
    Elvish,
    Powershell,
}

impl CompletionsArgs {
    pub fn run(self) -> Exit {
        match self.shell {
            Shell::Bash => print(Builtin::Bash),
            Shell::Zsh => print(Builtin::Zsh),
            Shell::Fish => print(Builtin::Fish),
            Shell::Nushell => print(Nushell),
            Shell::Elvish => print(Builtin::Elvish),
            Shell::Powershell => print(Builtin::PowerShell),
        }
        Exit::Success
    }
}

fn print(generator: impl Generator) {
    clap_complete::generate(
        generator,
        &mut MainCLIArgs::command(),
        "coeus",
        &mut std::io::stdout(),
    );
}
