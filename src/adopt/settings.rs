//! the daemon settings a job needs, read from the machine's own
//! configuration in the snapshot rather than from flags, so the first job is
//! guarded exactly as coeusd will guard every later one

use std::{
    net::{Ipv4Addr, SocketAddr},
    time::Duration,
};

use eyre::{Result, WrapErr, bail, eyre};
use serde::Deserialize;
use tokio::{net::TcpStream, process::Command, time};

use crate::{
    common::model::{Hostname, Platform, StorePath},
    config::DaemonSettings,
};

/// `services.coeus` as the module renders it into `coeusd.toml`: its
/// options under their TOML names, `settings` merged over them, nulls left
/// to coeusd's defaults. the key is left out, since a job never reads it.
pub const RENDER: &str = r#"cfg:
let
  clean = value:
    if builtins.isAttrs value then
      builtins.mapAttrs (_: clean) (
        builtins.removeAttrs value (builtins.filter (name: value.${name} == null) (builtins.attrNames value))
      )
    else
      value;
  merge = base: over:
    base // builtins.mapAttrs (
      name: value:
        if builtins.isAttrs value && builtins.isAttrs (base.${name} or null) then merge base.${name} value else value
    ) over;
in
{
  inherit (cfg) enable;
  settings =
    if cfg.enable then
      clean (merge {
        ssh_key_file = cfg.sshKeyFile;
        inherit (cfg) flake hostname listen history follow guard notify;
      } cfg.settings)
    else
      null;
}"#;

/// what [RENDER] evaluates to
#[derive(Debug, Deserialize)]
struct Rendered {
    enable: bool,
    settings: Option<DaemonSettings>,
}

/// the settings coeusd will run with once `hostname`'s configuration in
/// `source` is active. a configuration that does not enable coeus is
/// refused, since its coeusd would never answer the guard.
pub async fn of(source: &StorePath, hostname: &Hostname) -> Result<DaemonSettings> {
    let installable = format!(
        "path:{source}#{}.{}.config.services.coeus",
        Platform::current().configurations(),
        attribute(hostname),
    );

    let output = Command::new("nix")
        .args(["--extra-experimental-features", "nix-command flakes"])
        .args(["eval", "--json", "--no-write-lock-file", &installable])
        .args(["--apply", RENDER])
        .output()
        .await
        .wrap_err("failed to run nix")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!(
            "cannot read services.coeus of {hostname}: {}",
            tail(&stderr)
        );
    }
    parse(&output.stdout, hostname)
}

pub fn parse(json: &[u8], hostname: &Hostname) -> Result<DaemonSettings> {
    let rendered: Rendered =
        serde_json::from_slice(json).wrap_err("unexpected services.coeus settings")?;

    match (rendered.enable, rendered.settings) {
        (true, Some(settings)) => Ok(settings),
        _ => Err(eyre!(
            "{hostname}'s configuration does not enable services.coeus"
        )),
    }
}

/// a hostname as a nix attribute name
fn attribute(hostname: &Hostname) -> String {
    let name = hostname.as_str();
    let plain = name
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'));
    match plain {
        true => name.to_owned(),
        false => format!("\"{name}\""),
    }
}

/// nix's last error lines, which name what went wrong
fn tail(stderr: &str) -> String {
    let lines: Vec<&str> = stderr.trim().lines().collect();
    lines[lines.len().saturating_sub(12)..].join("\n")
}

/// whether something accepts connections on coeusd's port here
pub async fn listening(port: u16) -> bool {
    let address = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    time::timeout(Duration::from_secs(2), TcpStream::connect(address))
        .await
        .is_ok_and(|connected| connected.is_ok())
}
