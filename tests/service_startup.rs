//! Native compiled entrypoint fixtures only: ephemeral state/loopback, no deployments.
#![cfg(unix)]
use eyre::Result;
use nix::{
    sys::signal::{Signal, kill},
    unistd::{Pid, geteuid},
};
use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    path::Path,
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::Command,
    time::timeout,
};

fn command(home: &Path, state: &Path, config: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_coeusd"));
    command
        .arg("--service-home")
        .arg(home)
        .arg("--service-state-directory")
        .arg(state)
        .arg("--config")
        .arg(config)
        .env("NO_COLOR", "1")
        // Native startup must not rely on shell commands or external stat/id.
        .env("PATH", "/coeus-fixture-no-executables")
        .kill_on_drop(true);
    command
}

#[tokio::test]
async fn compiled_native_startup_accepts_secret_symlink_and_uses_private_working_directory()
-> Result<()> {
    if geteuid().is_root() {
        return Ok(());
    }
    let dir = tempfile::tempdir()?;
    let home = dir.path().join("home");
    let state = home.join("state");
    fs::create_dir_all(&state)?;
    fs::set_permissions(&state, fs::Permissions::from_mode(0o700))?;
    let actual_config = home.join("secret.toml");
    fs::write(
        &actual_config,
        format!(
            "[settings]\nkey = \"{}\"\nhost = \"127.0.0.1\"\nport = 0\nrepo_url = \"https://example.invalid/never-fetch\"\nstate_directory = \"ledger\"\n",
            "42".repeat(32)
        ),
    )?;
    fs::set_permissions(&actual_config, fs::Permissions::from_mode(0o400))?;
    let config = home.join("config-link");
    symlink(&actual_config, &config)?;
    let mut child = command(&home, &state, &config)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::inherit())
        .spawn()?;
    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
    timeout(Duration::from_secs(10), async {
        while let Some(line) = lines.next_line().await? {
            if line.contains("coeus server listening on 127.0.0.1:") {
                return Ok::<_, eyre::Report>(());
            }
        }
        eyre::bail!("daemon exited before native startup completed");
    })
    .await??;
    assert!(state.join("ledger").is_dir());
    assert!(!home.join("ledger").exists());
    #[cfg(target_os = "linux")]
    {
        let status = fs::read_to_string(format!("/proc/{}/status", child.id().unwrap()))?;
        assert!(status.lines().any(|line| line == "Umask:\t0077"));
    }
    kill(Pid::from_raw(child.id().unwrap() as i32), Signal::SIGTERM)?;
    let status = timeout(Duration::from_secs(10), child.wait()).await??;
    assert!(status.success());
    // No work was admitted: startup must not manufacture a job/activation receipt.
    assert!(
        !fs::read_dir(state.join("ledger"))?
            .filter_map(Result::ok)
            .any(|entry| entry.path().extension().is_some_and(|ext| ext == "job"))
    );
    Ok(())
}

#[tokio::test]
async fn compiled_native_startup_rejects_before_config_creation_or_ledger_access() -> Result<()> {
    if geteuid().is_root() {
        return Ok(());
    }
    let dir = tempfile::tempdir()?;
    let home = dir.path().join("home");
    let state = home.join("state");
    fs::create_dir_all(&state)?;
    fs::set_permissions(&state, fs::Permissions::from_mode(0o744))?;
    let config = home.join("missing.toml");
    let output = timeout(
        Duration::from_secs(10),
        command(&home, &state, &config).output(),
    )
    .await??;
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("mode 0700"));
    assert!(!config.exists());
    assert_eq!(fs::read_dir(&state)?.count(), 0);
    fs::set_permissions(&state, fs::Permissions::from_mode(0o700))?;
    let output = timeout(
        Duration::from_secs(10),
        command(&home, &state, &config).output(),
    )
    .await??;
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("cannot resolve Coeus service config")
    );
    assert!(!config.exists());
    fs::write(&config, "invalid content must not be parsed")?;
    fs::set_permissions(&config, fs::Permissions::from_mode(0o644))?;
    let output = timeout(
        Duration::from_secs(10),
        command(&home, &state, &config).output(),
    )
    .await??;
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("0600 or 0400"));
    assert_eq!(fs::read_dir(&state)?.count(), 0);
    Ok(())
}

#[test]
fn service_flags_must_be_paired_and_ordinary_cli_remains_valid() {
    use clap::Parser;
    use coeus::cli::daemon::DaemonCLIArgs;
    assert!(DaemonCLIArgs::try_parse_from(["coeusd", "--service-home", "/home"]).is_err());
    assert!(
        DaemonCLIArgs::try_parse_from(["coeusd", "--service-state-directory", "/state"]).is_err()
    );
    let args = DaemonCLIArgs::try_parse_from(["coeusd", "--config", "/config"]).unwrap();
    assert!(args.service_home.is_none() && args.service_state_directory.is_none());
}

#[tokio::test]
async fn compiled_native_startup_loads_external_runtime_key_without_writeback() -> Result<()> {
    if geteuid().is_root() {
        return Ok(());
    }
    let dir = tempfile::tempdir()?;
    let home = dir.path().join("home");
    let state = home.join("state");
    fs::create_dir_all(&state)?;
    fs::set_permissions(&state, fs::Permissions::from_mode(0o700))?;
    let key = home.join("psk");
    fs::write(&key, format!("{}\n", "42".repeat(32)))?;
    fs::set_permissions(&key, fs::Permissions::from_mode(0o400))?;
    let config = home.join("server.toml");
    let raw = format!(
        "[settings]\nkey_file = {key:?}\nhost = \"127.0.0.1\"\nport = 0\nrepo_url = \"https://example.invalid/never-fetch\"\nstate_directory = \"ledger\"\n"
    );
    fs::write(&config, &raw)?;
    fs::set_permissions(&config, fs::Permissions::from_mode(0o400))?;
    let mut child = command(&home, &state, &config)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::inherit())
        .spawn()?;
    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
    timeout(Duration::from_secs(10), async {
        while let Some(line) = lines.next_line().await? {
            assert!(!line.contains(&"42".repeat(32)));
            if line.contains("coeus server listening on 127.0.0.1:") {
                return Ok::<_, eyre::Report>(());
            }
        }
        eyre::bail!("daemon did not start with external key");
    })
    .await??;
    assert!(state.join("ledger").is_dir());
    assert_eq!(fs::read_to_string(&config)?, raw);
    kill(Pid::from_raw(child.id().unwrap() as i32), Signal::SIGTERM)?;
    assert!(
        timeout(Duration::from_secs(10), child.wait())
            .await??
            .success()
    );
    Ok(())
}

#[tokio::test]
async fn compiled_native_startup_rejects_missing_or_public_key_before_ledger_and_redacts()
-> Result<()> {
    if geteuid().is_root() {
        return Ok(());
    }
    let dir = tempfile::tempdir()?;
    let home = dir.path().join("home");
    let state = home.join("state");
    fs::create_dir_all(&state)?;
    fs::set_permissions(&state, fs::Permissions::from_mode(0o700))?;
    let key = home.join("missing-key");
    let config = home.join("server.toml");
    let raw = format!(
        "[settings]\nkey_file = {key:?}\nhost = \"127.0.0.1\"\nrepo_url = \"https://example.invalid/never-fetch\"\n"
    );
    fs::write(&config, &raw)?;
    fs::set_permissions(&config, fs::Permissions::from_mode(0o600))?;
    for provision in [false, true] {
        if provision {
            fs::write(&key, "42".repeat(32))?;
            fs::set_permissions(&key, fs::Permissions::from_mode(0o644))?;
        }
        let output = timeout(
            Duration::from_secs(10),
            command(&home, &state, &config).output(),
        )
        .await??;
        assert!(!output.status.success());
        assert!(!String::from_utf8_lossy(&output.stdout).contains(&"42".repeat(32)));
        assert!(!String::from_utf8_lossy(&output.stderr).contains(&"42".repeat(32)));
        assert_eq!(fs::read_to_string(&config)?, raw);
        assert_eq!(fs::read_dir(&state)?.count(), 0);
    }
    Ok(())
}
