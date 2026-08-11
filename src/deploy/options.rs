use clap::Args;
use std::path::PathBuf;

#[derive(Args, Debug, Clone, Default)]
pub struct DeployOptions {
    /// Local hostname to deploy to. Defaults to the remote host, then `HOSTNAME`.
    #[arg(long, value_name = "HOSTNAME")]
    hostname: Option<String>,

    /// Boot instead of switch (NixOS only)
    #[arg(short, long, value_name = "BOOT", default_value_t = false)]
    pub boot: bool,

    /// Sets "eval-cache" to false in nix options
    #[arg(short, long, value_name = "DRY_RUN", default_value_t = false)]
    pub dry_run: bool,

    /// Initial setup for new host
    #[arg(short, long, value_name = "INITIAL", default_value_t = false)]
    pub initial: bool,

    /// Ignore stale substituters from the current system
    #[arg(long, value_name = "CLEAN_SUBSTITUTERS")]
    pub clean_substituters: bool,

    /// Remote host address or hostname
    #[arg(short, long, value_name = "REMOTE")]
    pub remote: Option<String>,

    /// SSH identity file for remote access
    #[arg(short, long, value_name = "SSH_KEY")]
    pub ssh_key: Option<PathBuf>,

    /// Only build, don't apply
    #[arg(long, value_name = "BUILD_ONLY")]
    pub build_only: bool,

    /// Only apply, don't build
    #[arg(long, value_name = "APPLY_ONLY")]
    pub apply_only: bool,
}

impl DeployOptions {
    pub fn hostname(&self) -> &str {
        self.hostname
            .as_deref()
            .or(self.remote.as_deref())
            .unwrap_or(env!("HOSTNAME"))
    }
}
