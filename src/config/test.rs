use std::path::Path;

use easy_config_store::ConfigStore;

use super::*;
use crate::common::model::Hostname;

fn example<T>(name: &str) -> T
where
    T: Default + serde::Serialize + serde::de::DeserializeOwned + PartialEq + Clone,
{
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(name);
    (*ConfigStore::<T>::read(path, "settings".to_owned()).unwrap()).clone()
}

#[test]
fn the_daemon_example_parses() {
    let settings: DaemonSettings = example("coeusd.example.toml");

    assert_eq!(settings.flake, "git+https://github.com/GustavoWidman/nix");
    assert_eq!(settings.listen.port(), DEFAULT_PORT);
    assert_eq!(settings.follow.unwrap().git_ref, "deployed");
    assert_eq!(settings.guard.timeout.as_secs(), 120);
    assert!(settings.notify.is_none());
}

#[test]
fn the_client_example_parses_into_an_inventory() {
    let settings: ClientSettings = example("coeus.example.toml");
    let oracle: Hostname = "oracle-1".parse().unwrap();
    let unknown: Hostname = "phone".parse().unwrap();

    assert_eq!(settings.hosts.len(), 3);
    assert_eq!(settings.hosts[&"lab".parse().unwrap()].tags, ["nixos", "dev-server", "canary"]);
    assert_eq!(settings.endpoint(&oracle).port, DEFAULT_PORT);
    assert_eq!(settings.endpoint(&unknown).host, "phone");
}

#[test]
fn keys_load_from_inline_values_and_files() {
    let key = Key::generate();
    let file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(file.path(), format!("{}\n", key.to_hex())).unwrap();

    let inline = KeySource {
        key: Some(key.clone()),
        key_file: None,
    };
    let from_file = KeySource {
        key: None,
        key_file: Some(file.path().to_owned()),
    };

    assert_eq!(inline.load().unwrap(), key);
    assert_eq!(from_file.load().unwrap(), key);
    assert!("abc".parse::<Key>().is_err());
    assert_eq!(format!("{key:?}"), "Key(..)");
}
