#![cfg(unix)]
use super::*;
use std::{
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt, symlink},
};

const KEY: &str = "4242424242424242424242424242424242424242424242424242424242424242";

struct Fixture {
    dir: tempfile::TempDir,
    config: PathBuf,
    key: PathBuf,
    store: PathBuf,
    uid: u32,
}
impl Fixture {
    fn new() -> Result<Self> {
        let dir = tempfile::tempdir()?;
        let config = dir.path().join("server.toml");
        let key = dir.path().join("runtime-key");
        let store = dir.path().join("fixture-store");
        fs::create_dir(&store)?;
        fs::write(&key, KEY)?;
        fs::set_permissions(&key, fs::Permissions::from_mode(0o400))?;
        let uid = fs::metadata(&key)?.uid();
        let fixture = Self {
            dir,
            config,
            key,
            store,
            uid,
        };
        fixture.write(&format!("key_file = {:?}", fixture.key))?;
        Ok(fixture)
    }
    fn write(&self, credentials: &str) -> Result<()> {
        fs::write(
            &self.config,
            format!(
                "[settings]\n{credentials}\nhost = \"127.0.0.1\"\nrepo_url = \"https://example.invalid/nix\"\n"
            ),
        )?;
        fs::set_permissions(&self.config, fs::Permissions::from_mode(0o600))?;
        Ok(())
    }
    fn load(&self) -> Result<(PathBuf, ConfigInner)> {
        // Unprivileged synthetic store/owner; production fixes these to /nix/store + root.
        files::load_with_store(&self.config, self.uid, &self.store, self.uid)
    }
    fn public(&mut self) -> Result<()> {
        let path = self.store.join("public.toml");
        fs::rename(&self.config, &path)?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o444))?;
        self.config = path;
        Ok(())
    }
}

#[test]
fn immutable_loader_accepts_private_inline_and_external_key_without_writeback() -> Result<()> {
    let fixture = Fixture::new()?;
    let before = fs::read(&fixture.config)?;
    let (_, config) = fixture.load()?;
    assert_eq!(config.key, [0x42; 32]);
    assert_eq!(fs::read(&fixture.config)?, before);
    assert!(!format!("{config:?}").contains(KEY));
    fixture.write(&format!("key = \"{KEY}\""))?;
    assert_eq!(fixture.load()?.1, config);
    fs::set_permissions(&fixture.config, fs::Permissions::from_mode(0o400))?;
    assert_eq!(fixture.load()?.1.key, [0x42; 32]);
    Ok(())
}

#[test]
fn public_store_fixture_accepts_only_external_key_and_remains_unchanged() -> Result<()> {
    let mut fixture = Fixture::new()?;
    fixture.public()?;
    let before = fs::read(&fixture.config)?;
    assert_eq!(fixture.load()?.1.key, [0x42; 32]);
    assert_eq!(fs::read(&fixture.config)?, before);
    fs::set_permissions(&fixture.config, fs::Permissions::from_mode(0o644))?;
    assert!(fixture.load().is_err());
    fixture.write(&format!("key = \"{KEY}\""))?;
    fs::set_permissions(&fixture.config, fs::Permissions::from_mode(0o444))?;
    let error = fixture.load().unwrap_err().to_string();
    assert!(error.contains("inline PSK is forbidden"));
    assert!(!error.contains(KEY));
    assert!(
        files::load_with_store(
            &fixture.config,
            fixture.uid,
            &fixture.store,
            fixture.uid + 1
        )
        .is_err()
    );
    Ok(())
}

#[test]
fn rejects_both_neither_relative_missing_and_store_key_sources() -> Result<()> {
    let fixture = Fixture::new()?;
    for source in [
        String::new(),
        format!("key = \"{KEY}\"\nkey_file = {:?}", fixture.key),
        "key_file = \"relative-key\"".into(),
        format!("key_file = {:?}", fixture.dir.path().join("missing")),
    ] {
        fixture.write(&source)?;
        assert!(fixture.load().is_err());
    }
    let store_key = fixture.store.join("key");
    fs::copy(&fixture.key, &store_key)?;
    fixture.write(&format!("key_file = {store_key:?}"))?;
    assert!(
        fixture
            .load()
            .unwrap_err()
            .to_string()
            .contains("outside the Nix store")
    );
    let link = fixture.dir.path().join("store-key-link");
    symlink(&store_key, &link)?;
    fixture.write(&format!("key_file = {link:?}"))?;
    assert!(fixture.load().is_err());
    Ok(())
}

#[test]
fn accepts_runtime_secret_links_and_one_optional_lf_only() -> Result<()> {
    let fixture = Fixture::new()?;
    let link = fixture.dir.path().join("runtime-key-link");
    symlink(&fixture.key, &link)?;
    fixture.write(&format!("key_file = {link:?}"))?;
    fs::set_permissions(&fixture.key, fs::Permissions::from_mode(0o600))?;
    fs::write(&fixture.key, format!("{KEY}\n"))?;
    assert_eq!(fixture.load()?.1.key, [0x42; 32]);
    let config_link = fixture.dir.path().join("config-link");
    symlink(&fixture.config, &config_link)?;
    assert_eq!(files::load(&config_link, fixture.uid)?.1.key, [0x42; 32]);
    for invalid in [
        format!("{KEY}\n\n"),
        format!("{KEY}\r\n"),
        format!(" {KEY}"),
        "a".repeat(63),
        "é".repeat(32),
        "z".repeat(64),
        "a".repeat(10000),
    ] {
        fs::write(&fixture.key, &invalid)?;
        let error = fixture.load().unwrap_err().to_string();
        assert!(!error.contains(&invalid));
        assert!(!error.contains(KEY));
    }
    Ok(())
}

#[test]
fn rejects_wrong_owner_and_public_private_modes() -> Result<()> {
    let fixture = Fixture::new()?;
    assert!(
        files::load_with_store(
            &fixture.config,
            fixture.uid + 1,
            &fixture.store,
            fixture.uid
        )
        .is_err()
    );
    for mode in [0o644, 0o640, 0o660, 0o000] {
        fs::set_permissions(&fixture.key, fs::Permissions::from_mode(mode))?;
        assert!(fixture.load().is_err(), "key mode {mode:o}");
    }
    fs::set_permissions(&fixture.key, fs::Permissions::from_mode(0o400))?;
    for mode in [0o644, 0o640, 0o660, 0o000] {
        fs::set_permissions(&fixture.config, fs::Permissions::from_mode(mode))?;
        assert!(fixture.load().is_err(), "config mode {mode:o}");
    }
    Ok(())
}

#[test]
fn missing_or_invalid_config_is_never_created_or_replaced_and_errors_redact_source() -> Result<()> {
    let fixture = Fixture::new()?;
    fs::remove_file(&fixture.config)?;
    assert!(server_config(fixture.config.clone()).is_err());
    assert!(!fixture.config.exists());
    fixture.write(&format!("key = \"{KEY}\"\nunknown = ["))?;
    let before = fs::read(&fixture.config)?;
    let error = fixture.load().unwrap_err().to_string();
    assert!(error.contains("details suppressed"));
    assert!(!error.contains(KEY));
    assert_eq!(fs::read(&fixture.config)?, before);
    fs::write(&fixture.config, "a".repeat(65537))?;
    assert!(
        fixture
            .load()
            .unwrap_err()
            .to_string()
            .contains("size limit")
    );
    Ok(())
}

#[test]
fn rejects_nonregular_key_and_config_without_hanging_or_following_replaced_final_link() -> Result<()>
{
    use nix::sys::stat::Mode;
    let fixture = Fixture::new()?;
    fs::remove_file(&fixture.key)?;
    nix::unistd::mkfifo(&fixture.key, Mode::from_bits_truncate(0o600))?;
    assert!(
        fixture
            .load()
            .unwrap_err()
            .to_string()
            .contains("regular file")
    );
    fs::remove_file(&fixture.key)?;
    fs::create_dir(&fixture.key)?;
    assert!(fixture.load().is_err());
    fixture.write("key_file = \"/dev/null\"")?;
    assert!(fixture.load().is_err());
    fs::remove_file(&fixture.config)?;
    nix::unistd::mkfifo(&fixture.config, Mode::from_bits_truncate(0o600))?;
    assert!(fixture.load().is_err());
    fs::remove_file(&fixture.config)?;
    symlink("/etc/passwd", &fixture.config)?;
    // Direct resolved-open step models replacement after canonicalization.
    assert!(files::open_resolved(&fixture.config, "fixture config").is_err());
    Ok(())
}

#[test]
fn private_mode_policy_rejects_all_special_bits() {
    // Exercise the production predicate without privileged chmod in the sandbox.
    assert!(files::private_mode(0o100600));
    assert!(files::private_mode(0o100400));
    for mode in [0o1600, 0o2400, 0o2600, 0o4400, 0o4600] {
        assert!(!files::private_mode(mode));
    }
}
