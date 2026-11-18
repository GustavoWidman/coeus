use std::{path::Path, time::Duration};

use easy_config_store::ConfigStore;

use super::*;
use crate::common::{model::Hostname, proto::Role};

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
    let follow = settings.follow.unwrap();
    assert_eq!(follow.git_ref, "deployed");
    assert_eq!(
        follow.state_file.unwrap(),
        Path::new("/run/coeus/follow.json")
    );
    assert_eq!(settings.guard.timeout.as_secs(), 120);
    assert!(settings.notify.is_none());
}

#[test]
fn a_follow_without_a_state_file_publishes_nothing() {
    let follow: FollowSettings = toml::from_str(r#"ref = "deployed""#).unwrap();
    assert_eq!(follow.state_file, None);
}

#[test]
fn the_client_example_parses_into_an_inventory() {
    let settings: ClientSettings = example("coeus.example.toml");
    let oracle: Hostname = "oracle-1".parse().unwrap();
    let unknown: Hostname = "phone".parse().unwrap();

    assert_eq!(settings.hosts.len(), 3);
    assert_eq!(
        settings.hosts[&"lab".parse().unwrap()].tags,
        ["nixos", "dev-server", "canary"]
    );
    assert_eq!(settings.endpoint(&oracle).port, DEFAULT_PORT);
    assert_eq!(settings.endpoint(&unknown).host, "phone");
}

#[test]
fn the_web_example_parses() {
    let settings =
        WebSettings::load(&Path::new(env!("CARGO_MANIFEST_DIR")).join("web.example.toml")).unwrap();

    assert_eq!(
        settings.client.key.key_file.unwrap(),
        Path::new("/run/agenix/coeus-read-key")
    );
    assert_eq!(settings.client.hosts.len(), 2);
    assert_eq!(
        settings.web.origin_socket,
        Path::new("/run/coeus-web/origin.sock")
    );
    let tunnel = settings.web.tunnel.unwrap();
    assert_eq!(
        tunnel.credentials_file,
        Path::new("/run/agenix/coeus-tunnel")
    );
    assert_eq!(tunnel.cloudflared, Path::new("cloudflared"));
    assert_eq!(settings.web.readiness, ReadinessSettings::default());
}

#[test]
fn a_web_config_without_a_tunnel_serves_no_one() {
    let file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(
        file.path(),
        "[settings]\nkey_file = \"/run/agenix/coeus-read-key\"\n[settings.web]\norigin_socket = \"/run/o.sock\"\n",
    )
    .unwrap();
    let settings = WebSettings::load(file.path()).unwrap();
    assert_eq!(settings.web.tunnel, None);
    assert_eq!(settings.web.readiness.interval, Duration::from_secs(2));
    assert_eq!(settings.web.readiness.up_after.get(), 3);
    assert_eq!(settings.web.readiness.down_after.get(), 5);
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

#[test]
fn a_daemon_accepts_a_read_only_key_beside_the_fleet_key() {
    let full = Key::generate();
    let read = Key::generate();
    let file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(file.path(), read.to_hex()).unwrap();
    let mut settings = DaemonSettings {
        key: KeySource {
            key: Some(full.clone()),
            key_file: None,
        },
        ..DaemonSettings::default()
    };

    assert_eq!(settings.keys().unwrap(), [(Role::Full, full.clone())]);

    settings.read_key_file = Some(file.path().to_owned());
    assert_eq!(
        settings.keys().unwrap(),
        [(Role::Full, full.clone()), (Role::Read, read)]
    );

    settings.read_key = Some(full);
    let error = settings.keys().unwrap_err();
    assert!(error.to_string().contains("is the fleet key"), "{error}");
}

#[test]
fn the_state_directory_is_created_private_and_named_by_its_real_path() {
    use std::os::unix::fs::PermissionsExt;

    let root = tempfile::tempdir().unwrap();
    let real = root.path().join("private");
    std::fs::create_dir(&real).unwrap();
    std::os::unix::fs::symlink(&real, root.path().join("var")).unwrap();

    let state_dir = create_state_dir(&root.path().join("var/lib/coeus")).unwrap();
    assert_eq!(
        state_dir,
        real.canonicalize().unwrap().join("lib/coeus"),
        "nix 2.34 will not read beneath a symlink"
    );
    let mode = std::fs::metadata(&state_dir).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o700);
}
