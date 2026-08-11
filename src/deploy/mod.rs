mod command;
pub mod options;
mod utils;

use eyre::Result;

use crate::deploy::{command::CommandRunner, options::DeployOptions};

const CLEAN_SUBSTITUTERS: &str = "https://cache.nixos.org https://nix-community.cachix.org https://r3dlust.cachix.org https://install.determinate.systems";
const CLEAN_TRUSTED_PUBLIC_KEYS: &str = "cache.flakehub.com-3:hJuILl5sVK4iKm86JzgdXW12Y2Hwd5G07qKtHTOcDCM= cache.nixos.org-1:6NCHdD59X431o0gWypbMrAURkbJ16ZPMQFGspcDShjY= nix-community.cachix.org-1:mB9FSh9qf2dCimDSUo8Zy7bkq5CX+/rkCWyvRCYg3Fs= r3dlust.cachix.org-1:/R3S8pW/nr7kOBJKcGPsZ0zCepvldTUEgbrqa4O3cW0=";
const SYSTEM: &str = if cfg!(target_os = "macos") {
    "darwin"
} else {
    "os"
};

pub async fn deploy(options: &DeployOptions) -> Result<()> {
    let mut runner = CommandRunner::new("nh")
        .args([SYSTEM, "build", "."])
        .arg("--quiet");

    // TODO detect when running in daemon mode
    if false {
        // depends on the sudoers configuration defined in the flake for coeusd
        runner = runner.args(["--elevation-strategy", "passwordless"]);
    }

    runner = runner
        .args(["--hostname", options.hostname()])
        .arg("--")
        .args(["--extra-experimental-features", "pipe-operators"])
        .args(["--option", "accept-flake-config", "true"]);

    if let Some(remote) = &options.remote {
        sync_files(options, remote).await?;

        runner = runner
            .ssh(
                remote,
                None,
                options
                    .ssh_key
                    .as_ref()
                    .map(|key| key.display().to_string())
                    .as_deref(),
            )
            .path("~/.nix");
    }

    let action = match (options.boot || options.initial) && SYSTEM == "os" {
        true => "boot",
        false => "switch",
    };

    if options.clean_substituters {
        runner = runner
            .args(["--option", "substituters", CLEAN_SUBSTITUTERS])
            .args(["--option", "extra-substituters", ""])
            .args(["--option", "trusted-public-keys", CLEAN_TRUSTED_PUBLIC_KEYS])
            .args(["--option", "extra-trusted-public-keys", ""]);
    }

    if options.dry_run {
        runner = runner.args(["--option", "eval-cache", "false"]);
    }

    if !options.apply_only {
        runner.clone().call().await?;
    }

    if !options.build_only {
        runner
            .replace_arg_at(1, action) // replace "build" with "switch" or "boot"
            .call()
            .await?;
    }

    Ok(())
}

async fn sync_files(options: &DeployOptions, remote: &str) -> Result<()> {
    // TODO replace with ferrisync
    CommandRunner::new("rsync")
        .args(vec![
            "--archive",
            "--compress",
            "--delete",
            "--recursive",
            "--force",
            "--delete-excluded",
            "--no-owner",
            "--no-group",
            "--rsh",
        ])
        .arg(utils::ssh_string(options.ssh_key.as_deref()))
        .arg("./")
        .arg(format!("{remote}:.nix/"))
        .call()
        .await

    // let action = if options.boot || options.initial {
    //     "--boot"
    // } else {
    //     ""
    // };
    // let mut nh_args = vec![
    //     "os",
    //     if options.build_only {
    //         "build"
    //     } else {
    //         "switch"
    //     },
    //     ".nix",
    //     "--hostname",
    //     options.hostname.as_str(),
    // ];
    // if !action.is_empty() {
    //     nh_args.push(action);
    // }

    // CommandRunner::new("nh")
    //     .ssh(
    //         remote,
    //         None,
    //         options
    //             .ssh_key
    //             .as_ref()
    //             .map(|key| key.display().to_string())
    //             .as_deref(),
    //     )
    //     .args(nh_args)
    //     .call()
    //     .await2
}
