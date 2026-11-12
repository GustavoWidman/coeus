use std::path::Path;

use crate::{
    bootstrap::state::{Ended, LastRun, Saved, StateDir},
    common::model::Timestamp,
};

#[test]
fn a_machine_keeps_everything_under_its_own_directory() {
    let state = StateDir::under(Path::new("/home/me/.local/state"), &"nova".parse().unwrap());
    let root = Path::new("/home/me/.local/state/coeus/bootstrap/nova");

    assert_eq!(state.root(), root);
    assert_eq!(state.saved(), root.join("bootstrap.toml"));
    assert_eq!(state.known_hosts(), root.join("known_hosts"));
    assert_eq!(state.session_key(), root.join("session_key"));
    assert_eq!(state.lock_file(), root.join("lock"));
    assert_eq!(state.control_path(), root.join("ssh-%r"));
    assert_eq!(state.clone_dir(), root.join("repo"));
}

#[test]
fn saved_inputs_round_trip_through_toml() {
    let mut saved = Saved {
        destination: Some("nixos@192.168.1.40".into()),
        repo: Some("/home/me/GitHub/nix".into()),
        last_run: Some(LastRun {
            started_at: Timestamp(1_794_443_502_123),
            ended: Some(Ended {
                at: Timestamp(1_794_443_602_123),
                how: "detached while building".into(),
            }),
        }),
        ..Saved::default()
    };
    saved.answers.insert("type".into(), "server".into());

    let text = toml::to_string(&saved).unwrap();
    assert_eq!(toml::from_str::<Saved>(&text).unwrap(), saved);
}

#[test]
fn an_empty_file_is_a_first_run() {
    assert_eq!(toml::from_str::<Saved>("").unwrap(), Saved::default());
}
