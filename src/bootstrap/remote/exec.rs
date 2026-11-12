//! a [Call] run on the machine through ssh, as root when it must be

use std::process::{ExitStatus, Output, Stdio};

use eyre::{Result, bail};
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, AsyncWriteExt, BufReader},
    process::Child,
    sync::mpsc,
};
use zeroize::Zeroizing;

use super::{Call, Privilege, Ssh, ssh::stderr_tail};

/// runs `call` and collects its output, whatever its status
pub async fn output(ssh: &Ssh, privilege: &Privilege, call: &Call) -> Result<Output> {
    let (argv, password) = privilege.wrap(call);
    let child = start(ssh, &argv, input(password, call.stdin()))?;
    Ok(child.wait_with_output().await?)
}

/// runs `call` and returns its stdout, or fails with the last line it said
pub async fn text(ssh: &Ssh, privilege: &Privilege, call: &Call) -> Result<String> {
    let output = output(ssh, privilege, call).await?;
    if !output.status.success() {
        let what = call.argv().into_iter().find(|word| !word.starts_with('-'));
        bail!(
            "{} on the machine failed: {}",
            what.unwrap_or_default(),
            stderr_tail(&output.stderr)
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// starts `call` with its stdout and stderr piped, for a caller that reads
/// them as they come
pub fn spawn(ssh: &Ssh, privilege: &Privilege, call: &Call) -> Result<Child> {
    let (argv, password) = privilege.wrap(call);
    start(ssh, &argv, input(password, call.stdin()))
}

/// runs `call`, handing each line of its stdout and stderr to `line` as
/// it comes, in the order they arrive
pub async fn follow(
    ssh: &Ssh,
    privilege: &Privilege,
    call: &Call,
    mut line: impl FnMut(&str),
) -> Result<ExitStatus> {
    let mut child = spawn(ssh, privilege, call)?;
    let (sender, mut lines) = mpsc::unbounded_channel();
    forward(child.stdout.take(), sender.clone());
    forward(child.stderr.take(), sender);
    while let Some(text) = lines.recv().await {
        line(&text);
    }
    Ok(child.wait().await?)
}

fn forward(
    output: Option<impl AsyncRead + Unpin + Send + 'static>,
    lines: mpsc::UnboundedSender<String>,
) {
    let Some(output) = output else { return };
    tokio::spawn(async move {
        let mut reader = BufReader::new(output).lines();
        while let Ok(Some(line)) = reader.next_line().await {
            if lines.send(line).is_err() {
                return;
            }
        }
    });
}

/// an argv that is not a [Call], for sudo's own checks
pub(super) async fn raw(ssh: &Ssh, argv: &[String], password: Option<&str>) -> Result<Output> {
    let child = start(ssh, argv, input(password, None))?;
    Ok(child.wait_with_output().await?)
}

/// the sudo password's line, then what the call reads
fn input(password: Option<&str>, stdin: Option<&[u8]>) -> Zeroizing<Vec<u8>> {
    let mut bytes = Zeroizing::new(Vec::new());
    if let Some(password) = password {
        bytes.extend_from_slice(password.as_bytes());
        bytes.push(b'\n');
    }
    bytes.extend_from_slice(stdin.unwrap_or_default());
    bytes
}

fn start(ssh: &Ssh, argv: &[String], input: Zeroizing<Vec<u8>>) -> Result<Child> {
    let mut child = ssh
        .command(argv)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut stdin = child.stdin.take().expect("stdin is piped");
    tokio::spawn(async move {
        let _ = stdin.write_all(&input).await;
        let _ = stdin.shutdown().await;
    });
    Ok(child)
}
