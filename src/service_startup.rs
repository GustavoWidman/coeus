//! Module-managed startup checks, in-process so launchd starts the native daemon.
//! No config/state is created until these checks succeed. Both platform modules
//! and independent activation share immutable config/secret validation.
use eyre::{Context, Result, ensure};
use nix::{sys::stat::Mode, unistd::geteuid};
use std::{
    fs,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
};

/// Validate service files before applying the module's umask and working directory.
/// Called once at startup, before configuration loading or any server/ledger work.
pub fn prepare(home: &Path, state: &Path, config: &Path) -> Result<PathBuf> {
    let config = validate(home, state, config, geteuid().as_raw())?;
    nix::sys::stat::umask(Mode::from_bits_truncate(0o077));
    std::env::set_current_dir(state).wrap_err("cannot enter Coeus service state directory")?;
    Ok(config)
}

fn validate(home: &Path, state: &Path, config: &Path, uid: u32) -> Result<PathBuf> {
    ensure!(uid != 0, "coeusd must not run as root");
    ensure!(
        home.is_absolute() && state.is_absolute() && config.is_absolute(),
        "Coeus service paths must be absolute"
    );
    let home_meta = fs::metadata(home).wrap_err("cannot inspect Coeus service home")?;
    ensure!(
        home_meta.is_dir() && home_meta.uid() == uid,
        "Coeus service home must be a directory owned by the service user"
    );
    let state_meta =
        fs::metadata(state).wrap_err("cannot inspect Coeus service state directory")?;
    ensure!(
        state_meta.is_dir() && state_meta.uid() == uid && state_meta.mode() & 0o7777 == 0o700,
        "Coeus service state must be a service-owned directory with mode 0700"
    );
    // The same immutable loader validates private config or public store TOML
    // plus its external private key before state/bind work on both platforms.
    let (config, _) = crate::config::server::files_for_startup(config, uid)?;
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{PermissionsExt, symlink};

    struct Fixture {
        _dir: tempfile::TempDir,
        home: PathBuf,
        state: PathBuf,
        config: PathBuf,
        uid: u32,
    }
    impl Fixture {
        fn new() -> Result<Self> {
            let dir = tempfile::tempdir()?;
            let home = dir.path().join("home");
            let state = home.join("state");
            let config = home.join("config.toml");
            fs::create_dir_all(&state)?;
            fs::set_permissions(&state, fs::Permissions::from_mode(0o700))?;
            fs::write(
                &config,
                format!(
                    "[settings]\nkey = \"{}\"\nhost = \"127.0.0.1\"\nrepo_url = \"https://example.invalid/nix\"\n",
                    "42".repeat(32)
                ),
            )?;
            fs::set_permissions(&config, fs::Permissions::from_mode(0o600))?;
            let uid = fs::metadata(&config)?.uid();
            Ok(Self {
                _dir: dir,
                home,
                state,
                config,
                uid,
            })
        }
        fn validate(&self) -> Result<PathBuf> {
            validate(&self.home, &self.state, &self.config, self.uid)
        }
    }

    #[test]
    fn accepts_private_config_and_resolved_secret_link() -> Result<()> {
        let fixture = Fixture::new()?;
        // The production policy deliberately forbids a root daemon.
        if fixture.uid == 0 {
            return Ok(());
        }
        assert_eq!(fixture.validate()?, fs::canonicalize(&fixture.config)?);
        fs::set_permissions(&fixture.config, fs::Permissions::from_mode(0o400))?;
        let link = fixture.home.join("secret-link");
        symlink(&fixture.config, &link)?;
        assert_eq!(
            validate(&fixture.home, &fixture.state, &link, fixture.uid)?,
            fs::canonicalize(&fixture.config)?
        );
        Ok(())
    }

    #[test]
    fn rejects_root_relative_paths_and_wrong_owner() -> Result<()> {
        let fixture = Fixture::new()?;
        assert!(
            validate(&fixture.home, &fixture.state, &fixture.config, 0)
                .unwrap_err()
                .to_string()
                .contains("root")
        );
        if fixture.uid == 0 {
            return Ok(());
        }
        assert!(
            validate(
                Path::new("relative"),
                &fixture.state,
                &fixture.config,
                fixture.uid
            )
            .is_err()
        );
        assert!(
            validate(
                &fixture.home,
                &fixture.state,
                &fixture.config,
                fixture.uid + 1
            )
            .is_err()
        );
        Ok(())
    }

    #[test]
    fn rejects_missing_or_non_directory_home_and_state() -> Result<()> {
        let fixture = Fixture::new()?;
        if fixture.uid == 0 {
            return Ok(());
        }
        assert!(
            validate(
                &fixture.config,
                &fixture.state,
                &fixture.config,
                fixture.uid
            )
            .is_err()
        );
        assert!(validate(&fixture.home, &fixture.config, &fixture.config, fixture.uid).is_err());
        fs::remove_dir(&fixture.state)?;
        assert!(fixture.validate().is_err());
        fs::remove_file(&fixture.config)?;
        fs::remove_dir(&fixture.home)?;
        assert!(fixture.validate().is_err());
        Ok(())
    }

    #[test]
    fn rejects_non_private_state_and_config_modes_including_special_bits() -> Result<()> {
        let fixture = Fixture::new()?;
        if fixture.uid == 0 {
            return Ok(());
        }
        for mode in [0o744, 0o750, 0o770, 0o1700] {
            fs::set_permissions(&fixture.state, fs::Permissions::from_mode(mode))?;
            assert!(fixture.validate().is_err(), "state mode {mode:o}");
        }
        fs::set_permissions(&fixture.state, fs::Permissions::from_mode(0o700))?;
        for mode in [0o644, 0o640, 0o660, 0o000, 0o1600] {
            fs::set_permissions(&fixture.config, fs::Permissions::from_mode(mode))?;
            assert!(fixture.validate().is_err(), "config mode {mode:o}");
        }
        Ok(())
    }

    #[test]
    fn rejects_other_owned_config_even_when_readable() -> Result<()> {
        let fixture = Fixture::new()?;
        if fixture.uid == 0 {
            return Ok(());
        }
        // Both supported Unix platforms provide a root-owned readable passwd.
        let other = Path::new("/etc/passwd");
        assert_ne!(fs::metadata(other)?.uid(), fixture.uid);
        assert!(
            validate(&fixture.home, &fixture.state, other, fixture.uid)
                .unwrap_err()
                .to_string()
                .contains("owned by the service user")
        );
        Ok(())
    }

    #[test]
    fn rejects_missing_config_and_directory_config() -> Result<()> {
        let fixture = Fixture::new()?;
        if fixture.uid == 0 {
            return Ok(());
        }
        fs::remove_file(&fixture.config)?;
        assert!(fixture.validate().is_err());
        fs::create_dir(&fixture.config)?;
        assert!(fixture.validate().is_err());
        Ok(())
    }

    #[test]
    fn rejects_fifo_config_without_blocking() -> Result<()> {
        let fixture = Fixture::new()?;
        if fixture.uid == 0 {
            return Ok(());
        }
        fs::remove_file(&fixture.config)?;
        nix::unistd::mkfifo(&fixture.config, Mode::from_bits_truncate(0o600))?;
        assert!(
            fixture
                .validate()
                .unwrap_err()
                .to_string()
                .contains("regular file")
        );
        Ok(())
    }

    #[test]
    fn rejects_config_resolving_into_store() -> Result<()> {
        let fixture = Fixture::new()?;
        if fixture.uid == 0 {
            return Ok(());
        }
        // A real existing store path, without needing permission to create one.
        if Path::new("/nix/store").is_dir() {
            fs::remove_file(&fixture.config)?;
            symlink("/nix/store", &fixture.config)?;
            assert!(
                fixture
                    .validate()
                    .unwrap_err()
                    .to_string()
                    .contains("regular file")
            );
        }
        Ok(())
    }
}
