//! Compiled client diagnostics are concise by default; no real endpoint/service.
use eyre::Result;
use std::process::Command;

#[test]
fn default_error_is_plain_actionable_stderr_without_rust_location_or_secret() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let absent = dir.path().join("missing-client-config");
    let output = Command::new(env!("CARGO_BIN_EXE_coeus"))
        .args(["--config", absent.to_str().unwrap(), "status", "me"])
        .env("NO_COLOR", "1")
        .env("RUST_BACKTRACE", "1")
        .output()?;
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let text = String::from_utf8_lossy(&output.stderr);
    assert!(text.starts_with("error: "), "{text}");
    assert!(text.contains("config"), "{text}");
    assert!(
        !text.contains("Location:") && !text.contains("Stack backtrace") && !text.contains("src/"),
        "{text}"
    );
    assert!(!text.contains('\x1b') && !text.contains('—') && !text.contains('–'));
    assert!(text.lines().count() <= 2, "{text}");
    Ok(())
}

#[test]
fn explicit_debug_preserves_developer_diagnostics_without_polluting_stdout() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let output = Command::new(env!("CARGO_BIN_EXE_coeus"))
        .args([
            "--verbosity",
            "debug",
            "--config",
            dir.path().join("absent").to_str().unwrap(),
            "status",
            "me",
        ])
        .env("NO_COLOR", "1")
        .output()?;
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let text = String::from_utf8_lossy(&output.stderr);
    assert!(
        text.contains("Location:") || text.contains("src/config/client.rs"),
        "{text}"
    );
    Ok(())
}

#[test]
fn invalid_config_diagnostics_never_echo_secret_source_even_with_debug() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let config = dir.path().join("client.toml");
    let secret_marker = "synthetic-private-key-must-not-appear";
    std::fs::write(&config, format!("[settings]\nkey='{secret_marker}'\n"))?;
    for verbosity in ["info", "debug"] {
        let output = Command::new(env!("CARGO_BIN_EXE_coeus"))
            .args([
                "--verbosity",
                verbosity,
                "--config",
                config.to_str().unwrap(),
                "status",
                "me",
            ])
            .env("NO_COLOR", "1")
            .output()?;
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        let text = String::from_utf8_lossy(&output.stderr);
        assert!(text.contains("invalid client configuration"), "{text}");
        assert!(!text.contains(secret_marker), "{text}");
        assert!(!text.contains("key='"), "{text}");
        assert!(!text.contains('\x1b'), "{text}");
    }
    Ok(())
}
