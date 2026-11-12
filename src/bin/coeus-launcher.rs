//! the program macOS runners start under. it runs its arguments as a child
//! and waits, never replacing itself, so it stays the process macOS holds
//! responsible for everything below it, and the Full Disk Access granted to
//! it reaches nix-darwin's activation. coeusd installs it once and never
//! replaces it, which keeps the grant valid across coeus updates.

use std::{
    env,
    os::unix::process::ExitStatusExt,
    process::{Command, ExitCode},
};

fn main() -> ExitCode {
    let mut args = env::args_os().skip(1);
    let Some(program) = args.next() else {
        eprintln!("usage: coeus-launcher <program> [args...]");
        return ExitCode::from(64);
    };

    match Command::new(&program).args(args).status() {
        Ok(status) => match (status.code(), status.signal()) {
            (Some(code), _) => ExitCode::from(code as u8),
            (None, Some(signal)) => ExitCode::from(128 + signal as u8),
            (None, None) => ExitCode::FAILURE,
        },
        Err(error) => {
            eprintln!(
                "coeus-launcher: cannot run {}: {error}",
                program.to_string_lossy()
            );
            ExitCode::from(127)
        }
    }
}
