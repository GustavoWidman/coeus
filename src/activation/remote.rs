//! SSH is the transport, not the daemon. Never disable host-key verification.
use super::BinaryIdentity;
use clap::Args;
use eyre::{Result, WrapErr, bail, ensure};
use futures_util::future::BoxFuture;
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::{io::AsyncWriteExt, process::Command};

#[derive(Args, Debug, Clone)]
pub struct RemoteArgs {
    /// Existing SSH alias/user@host. Uses normal OpenSSH keys/agent and known_hosts.
    #[arg(long)]
    pub ssh: String,
    /// Private receipt directory on target, owned by the service/SSH user.
    #[arg(long)]
    pub activation_directory: PathBuf,
    /// Existing target-side server/client TOML; never copied or rewritten.
    #[arg(long)]
    pub peer_config: PathBuf,
    /// Explicit matching-OS/arch Coeus binary; defaults to this executable. No 'latest' fallback.
    #[arg(long)]
    pub copy_binary: Option<PathBuf>,
    /// Use operator's existing passwordless sudo for bootstrap/service restart; installs no rules.
    #[arg(long)]
    pub operator_sudo: bool,
}
#[derive(Args, Debug, Clone)]
pub struct LocalArgs {
    #[arg(long)]
    pub activation_directory: PathBuf,
    #[arg(long)]
    pub peer_config: PathBuf,
    #[arg(long)]
    pub operator_sudo: bool,
}

pub(crate) trait RemoteExecutor {
    fn script<'a>(&'a self, host: &'a str, script: String) -> BoxFuture<'a, Result<String>>;
    fn copy<'a>(
        &'a self,
        host: &'a str,
        source: &'a Path,
        directory: &'a str,
    ) -> BoxFuture<'a, Result<()>>;
}
struct Ssh;
impl RemoteExecutor for Ssh {
    fn script<'a>(&'a self, host: &'a str, script: String) -> BoxFuture<'a, Result<String>> {
        Box::pin(async move {
            let mut command = Command::new("ssh");
            command
                .args(ssh_arguments(host))
                .args(["/bin/sh", "-s"])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::inherit())
                .kill_on_drop(true);
            let mut process = crate::deploy::process::DeploymentProcess::spawn(&mut command)?;
            let mut input = process
                .child
                .stdin
                .take()
                .ok_or_else(|| eyre::eyre!("SSH stdin unavailable"))?;
            let mut output = process
                .child
                .stdout
                .take()
                .ok_or_else(|| eyre::eyre!("SSH stdout unavailable"))?;
            let send = async {
                input.write_all(script.as_bytes()).await?;
                input.shutdown().await?;
                drop(input);
                Ok::<_, eyre::Report>(())
            };
            let receive = async {
                use tokio::io::AsyncReadExt;
                let mut bytes = Vec::new();
                (&mut output)
                    .take(1024 * 1024 + 1)
                    .read_to_end(&mut bytes)
                    .await?;
                ensure!(bytes.len() <= 1024 * 1024, "SSH output exceeded bound");
                Ok::<_, eyre::Report>(bytes)
            };
            let (_, bytes, status) = tokio::time::timeout(Duration::from_secs(1800), async {
                tokio::try_join!(send, receive, async {
                    Ok::<_, eyre::Report>(process.child.wait().await?)
                })
            })
            .await
            .wrap_err("SSH deadline exceeded; inspect/reconcile target receipt, do not replay")??;
            ensure!(
                status.success(),
                "SSH operation failed ({status}); inspect target receipt before retrying"
            );
            Ok(String::from_utf8(bytes)?)
        })
    }
    fn copy<'a>(
        &'a self,
        host: &'a str,
        source: &'a Path,
        directory: &'a str,
    ) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            let mut command = Command::new("scp");
            command
                .args([
                    "-q",
                    "-o",
                    "BatchMode=yes",
                    "-o",
                    "StrictHostKeyChecking=yes",
                    "-o",
                    "ConnectTimeout=10",
                    "--",
                ])
                .arg(source)
                .arg(format!("{host}:{directory}/coeus"));
            let mut process = crate::deploy::process::DeploymentProcess::spawn(&mut command)?;
            let status =
                tokio::time::timeout(Duration::from_secs(120), process.child.wait()).await??;
            ensure!(status.success(), "copy of selected binary failed");
            Ok(())
        })
    }
}
pub(crate) fn ssh_arguments(host: &str) -> Vec<&str> {
    vec![
        "-T",
        "-o",
        "BatchMode=yes",
        "-o",
        "StrictHostKeyChecking=yes",
        "-o",
        "ConnectTimeout=10",
        "--",
        host,
    ]
}
pub(crate) fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}
fn path_argument(path: &Path) -> Result<String> {
    ensure!(path.is_absolute(), "target paths must be absolute");
    let text = path
        .to_str()
        .ok_or_else(|| eyre::eyre!("UTF8 target path required"))?;
    ensure!(
        !text.chars().any(char::is_control),
        "control character in path"
    );
    Ok(quote(text))
}
fn validate_host(host: &str) -> Result<()> {
    ensure!(
        !host.is_empty()
            && !host.starts_with('-')
            && host
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"@._-".contains(&b)),
        "use a simple SSH alias or user@host; configure ports/proxies in ~/.ssh/config"
    );
    Ok(())
}

pub async fn run_remote(
    args: &RemoteArgs,
    id: u64,
    bootstrap: Option<(&str, &str, &Path)>,
    reconcile_only: bool,
) -> Result<()> {
    remote_with(&Ssh, args, id, bootstrap, reconcile_only).await
}
pub(crate) async fn remote_with(
    executor: &dyn RemoteExecutor,
    args: &RemoteArgs,
    id: u64,
    bootstrap: Option<(&str, &str, &Path)>,
    reconcile_only: bool,
) -> Result<()> {
    validate_host(&args.ssh)?;
    ensure!(
        id != 0,
        "nonzero job ID required; reuse it to inspect a lost connection"
    );
    let directory = path_argument(&args.activation_directory)?;
    let peer = path_argument(&args.peer_config)?;
    if let Some((flake, host, nix)) = bootstrap {
        validate_flake(flake)?;
        validate_hostname(host)?;
        path_argument(nix)?;
    }
    let binary = args.copy_binary.clone().unwrap_or(std::env::current_exe()?);
    ensure!(
        binary.is_file(),
        "matching target Coeus binary unavailable; select --copy-binary. Exact-tag cross-target release fetch is not supported until portable releases exist"
    );
    // The shell command itself is constant. Arguments below only enter POSIX script stdin,
    // so the user's login shell (including Nushell) never interprets user input.
    let preflight = executor
        .script(
            &args.ssh,
            "set -eu\nuname -s\nuname -m\nmktemp -d /tmp/coeus.XXXXXXXXXX\n".into(),
        )
        .await?;
    let lines: Vec<_> = preflight.lines().collect();
    ensure!(lines.len() == 3, "unexpected SSH preflight output");
    let os = match lines[0] {
        "Linux" => "linux",
        "Darwin" => "macos",
        _ => {
            bail!("unsupported SSH target OS");
        }
    };
    let arch = match lines[1] {
        "x86_64" => "x86_64",
        "aarch64" | "arm64" => "aarch64",
        _ => {
            bail!("unsupported target architecture");
        }
    };
    let temp = lines[2];
    ensure!(
        temp.starts_with("/tmp/coeus.")
            && temp.len() < 80
            && temp
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"/._-".contains(&b)),
        "invalid remote temporary directory"
    );
    let operation = async {
        executor.copy(&args.ssh, &binary, temp).await?;
        let identity = executor.script(&args.ssh, format!("set -eu\nchmod 700 {temp}/coeus\n{temp}/coeus identity\n")).await?;
        let identity: BinaryIdentity = toml::from_str(&identity)?;
        let mut expected = BinaryIdentity::current(); expected.os = os.into(); expected.arch = arch.into();
        ensure!(identity == expected, "target binary version/protocol/OS/arch mismatch; choose an exact matching binary, never latest");
        let mode = if bootstrap.is_some() { "bootstrap-local" } else if reconcile_only { "reconcile-local" } else { "activate-local" };
        let mut script = format!("set -eu\n{temp}/coeus {mode} {id} --activation-directory {directory} --peer-config {peer}");
        if args.operator_sudo { script.push_str(" --operator-sudo"); }
        if let Some((flake, host, nix)) = bootstrap { script.push_str(&format!(" --flake {} --hostname {} --nix-binary {}", quote(flake), quote(host), path_argument(nix)?)); }
        script.push('\n');
        let output = executor.script(&args.ssh, script).await?;
        print!("{output}");
        Ok::<_, eyre::Report>(())
    }.await;
    // Never delete receipts, roots, configuration or credentials. Temporary executable only.
    let cleanup = executor
        .script(&args.ssh, format!("rm -f {temp}/coeus\nrmdir {temp}\n"))
        .await;
    operation?;
    cleanup?;
    Ok(())
}
pub(super) fn validate_hostname(host: &str) -> Result<()> {
    ensure!(
        !host.is_empty()
            && host
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b)),
        "invalid host attribute"
    );
    Ok(())
}
pub(super) fn validate_flake(flake: &str) -> Result<()> {
    let parts: Vec<_> = flake
        .strip_prefix("github:")
        .unwrap_or("")
        .split('/')
        .collect();
    ensure!(
        parts.len() == 3
            && parts[..2].iter().all(|s| !s.is_empty()
                && s.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b)))
            && matches!(parts[2].len(), 40 | 64)
            && parts[2].bytes().all(|b| b.is_ascii_hexdigit()),
        "bootstrap requires github:OWNER/REPO/FULL_COMMIT (no floating branches/tags)"
    );
    Ok(())
}
