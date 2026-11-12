use std::path::Path;

use crate::bootstrap::{
    SshKey,
    remote::{
        Ssh,
        ssh::{Destination, first_contact_args, known_key},
    },
    state::StateDir,
};

const KEY: &str =
    "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIBqyOKVkbPDCemEVQJa2aQiocqctG5AenH2AeYZEHd/L";

#[test]
fn root_replaces_the_user_of_every_destination_form() {
    for (destination, root) in [
        ("nixos@192.168.1.40", "root@192.168.1.40"),
        ("nova", "root@nova"),
        ("ssh://nixos@127.0.0.1:22330", "ssh://root@127.0.0.1:22330"),
        ("ssh://nova", "ssh://root@nova"),
        ("ssh://me@[::1]:22", "ssh://root@[::1]:22"),
    ] {
        assert_eq!(Destination::parse(destination).with_user("root"), root);
    }
}

#[test]
fn first_contact_authenticates_with_nothing_and_records_into_scratch() {
    let args = first_contact_args("nixos@nova", Path::new("/tmp/scratch/known_hosts"));
    let options: Vec<&str> = args
        .chunks(2)
        .filter(|pair| pair[0] == "-o")
        .map(|pair| pair[1].as_str())
        .collect();
    for option in [
        "BatchMode=yes",
        "PreferredAuthentications=none",
        "StrictHostKeyChecking=accept-new",
        "UserKnownHostsFile=/tmp/scratch/known_hosts",
        "GlobalKnownHostsFile=/dev/null",
        "HashKnownHosts=no",
        "KnownHostsCommand=none",
        "VerifyHostKeyDNS=no",
        "HostKeyAlgorithms=ssh-ed25519",
        "ControlPath=none",
    ] {
        assert!(options.contains(&option), "{option}");
    }
    assert_eq!(args[args.len() - 2..], ["nixos@nova", "true"]);
}

#[test]
fn a_known_hosts_line_is_read_by_its_alias() {
    let text = format!("other {KEY}\nnova {KEY} trailing\n");
    assert_eq!(known_key(&text, "nova"), Some(SshKey::parse(KEY).unwrap()));
    assert_eq!(known_key(&text, "absent"), None);
    assert_eq!(known_key("nova ssh-rsa notakey\n", "nova"), None);
}

#[test]
fn a_pin_survives_to_the_next_run() {
    let home = tempfile::tempdir().unwrap();
    let state = StateDir::under(home.path(), &"nova".parse().unwrap());
    assert_eq!(Ssh::pinned(&state).unwrap(), None);

    let key = SshKey::parse(KEY).unwrap();
    Ssh::pin(&state, &key).unwrap();
    assert_eq!(Ssh::pinned(&state).unwrap(), Some(key));
    assert_eq!(
        std::fs::read_to_string(state.known_hosts()).unwrap(),
        format!("nova {KEY}\n")
    );
}

#[test]
fn nix_goes_through_the_master_with_the_pin() {
    let home = tempfile::tempdir().unwrap();
    let state = StateDir::under(home.path(), &"nova".parse().unwrap());
    let ssh = Ssh::login(&state, "ssh://nixos@127.0.0.1:22330");
    let (url, options) = ssh.nix_store("/run/current-system/sw/bin/nix-daemon");

    assert_eq!(
        url,
        "ssh-ng://nixos@127.0.0.1?max-connections=1&remote-program=/run/current-system/sw/bin/nix-daemon"
    );
    let control = format!("ControlPath={}", state.control_path().display());
    for option in [
        "HostKeyAlias=nova",
        "GlobalKnownHostsFile=/dev/null",
        "KnownHostsCommand=none",
        "VerifyHostKeyDNS=no",
        "StrictHostKeyChecking=yes",
        "HostKeyAlgorithms=ssh-ed25519",
        "BatchMode=yes",
        "ControlPersist=120",
        &control,
    ] {
        assert!(options.contains(option), "{option} in {options}");
    }
    assert!(options.ends_with("-p 22330"), "{options}");
}

#[test]
fn a_remote_program_with_arguments_survives_the_store_url() {
    let home = tempfile::tempdir().unwrap();
    let state = StateDir::under(home.path(), &"nova".parse().unwrap());
    let ssh = Ssh::login(&state, "nixos@192.168.1.40");
    let (url, _) = ssh.nix_store("sudo -n /run/current-system/sw/bin/nix-daemon");

    assert_eq!(
        url,
        "ssh-ng://nixos@192.168.1.40?max-connections=1&remote-program=sudo%20-n%20/run/current-system/sw/bin/nix-daemon"
    );
}
