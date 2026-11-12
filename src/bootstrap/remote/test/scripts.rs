//! the shell scripts of [Call], run on this machine against a scratch
//! directory standing in for root's home and `/etc`

use std::{fs, os::unix::fs::symlink, path::Path, process::Command};

use crate::bootstrap::{SshKey, remote::Call};

const KEY: &str =
    "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIBqyOKVkbPDCemEVQJa2aQiocqctG5AenH2AeYZEHd/L";
const OTHER: &str =
    "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIN9FLpE3q6S85tUkrfN6U7VMxHTkk6ikkPnNkS9uU7qx altar";

/// [scratched] `call`, run here
fn run(call: &Call, scratch: &Path) -> std::process::Output {
    let argv = scratched(call, scratch);
    Command::new(&argv[0]).args(&argv[1..]).output().unwrap()
}

/// `call`'s argv with every path under `/etc` and root's home moved into
/// `scratch`
fn scratched(call: &Call, scratch: &Path) -> Vec<String> {
    let scratch = scratch.display().to_string();
    call.argv()
        .into_iter()
        .map(|word| {
            word.replace("~root", &scratch)
                .replace("/etc/", &format!("{scratch}/etc/"))
        })
        .collect()
}

fn authorized_keys(scratch: &Path) -> String {
    fs::read_to_string(scratch.join(".ssh/authorized_keys")).unwrap()
}

#[test]
fn the_session_key_is_added_once_and_removed_alone() {
    let scratch = tempfile::tempdir().unwrap();
    let key = SshKey::parse(KEY).unwrap();
    let install = Call::InstallSessionKey {
        key: key.clone(),
        host: "nova".parse().unwrap(),
    };

    fs::create_dir_all(scratch.path().join(".ssh")).unwrap();
    // a file without its last newline must not run into the new line
    fs::write(scratch.path().join(".ssh/authorized_keys"), OTHER).unwrap();
    for _ in 0..2 {
        assert!(run(&install, scratch.path()).status.success());
    }
    assert_eq!(
        authorized_keys(scratch.path()),
        format!("{OTHER}\n{KEY} coeus-bootstrap@nova\n")
    );

    let remove = Call::RemoveSessionKey { key };
    for _ in 0..2 {
        assert!(run(&remove, scratch.path()).status.success());
    }
    assert_eq!(authorized_keys(scratch.path()), format!("{OTHER}\n"));
}

#[test]
fn removing_the_session_key_keeps_the_file_when_the_copy_cannot_be_written() {
    let scratch = tempfile::tempdir().unwrap();
    let file = scratch.path().join(".ssh/authorized_keys");
    fs::create_dir_all(scratch.path().join(".ssh")).unwrap();
    let before = format!("{OTHER}\n{KEY} coeus-bootstrap@nova\n");
    fs::write(&file, &before).unwrap();

    // no byte may be written, as on a full disk: the copy's write fails
    let remove = Call::RemoveSessionKey {
        key: SshKey::parse(KEY).unwrap(),
    };
    let mut command = Command::new("sh");
    command.args(["-c", r#"trap "" XFSZ; ulimit -f 0; exec "$@""#, "sh"]);
    let output = command
        .args(scratched(&remove, scratch.path()))
        .output()
        .unwrap();

    assert!(!output.status.success(), "{output:?}");
    assert_eq!(authorized_keys(scratch.path()), before);
    assert!(!scratch.path().join(".ssh/authorized_keys.coeus").exists());
}

#[test]
fn the_session_key_makes_roots_ssh_directory_when_there_is_none() {
    let scratch = tempfile::tempdir().unwrap();
    let key = SshKey::parse(KEY).unwrap();
    let remove = Call::RemoveSessionKey { key: key.clone() };
    assert!(run(&remove, scratch.path()).status.success());

    let install = Call::InstallSessionKey {
        key,
        host: "nova".parse().unwrap(),
    };
    assert!(run(&install, scratch.path()).status.success());
    assert_eq!(
        authorized_keys(scratch.path()),
        format!("{KEY} coeus-bootstrap@nova\n")
    );
}

#[test]
fn shell_files_go_aside_unless_nix_darwin_wrote_them() {
    let scratch = tempfile::tempdir().unwrap();
    let etc = scratch.path().join("etc");
    fs::create_dir_all(etc.join("static")).unwrap();
    fs::write(etc.join("bashrc"), "# the installer's\n").unwrap();
    fs::write(etc.join("static/zshrc"), "# nix-darwin's\n").unwrap();
    symlink(etc.join("static/zshrc"), etc.join("zshrc")).unwrap();

    let output = run(&Call::MoveEtcAside, scratch.path());
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        format!(
            "{} to {}\n",
            etc.join("bashrc").display(),
            etc.join("bashrc.before-nix-darwin").display()
        )
    );
    assert!(!etc.join("bashrc").exists());
    assert_eq!(
        fs::read_to_string(etc.join("bashrc.before-nix-darwin")).unwrap(),
        "# the installer's\n"
    );
    assert!(etc.join("zshrc").is_symlink());

    // nothing left to move the second time
    let again = run(&Call::MoveEtcAside, scratch.path());
    assert!(again.status.success());
    assert!(again.stdout.is_empty());
}

#[test]
fn a_copy_already_aside_is_kept_and_the_new_file_goes_beside_it() {
    let scratch = tempfile::tempdir().unwrap();
    let etc = scratch.path().join("etc");
    fs::create_dir_all(&etc).unwrap();
    fs::write(etc.join("zprofile"), "# the installer's\n").unwrap();
    fs::write(etc.join("zprofile.before-nix-darwin"), "# apple's\n").unwrap();

    let output = run(&Call::MoveEtcAside, scratch.path());
    assert!(output.status.success(), "{output:?}");
    assert!(!etc.join("zprofile").exists());
    let aside = |name: &str| fs::read_to_string(etc.join(name)).unwrap();
    assert_eq!(aside("zprofile.before-nix-darwin"), "# apple's\n");
    assert_eq!(aside("zprofile.before-nix-darwin.1"), "# the installer's\n");

    // the same file again was already moved: it goes, a third copy is not made
    fs::write(etc.join("zprofile"), "# the installer's\n").unwrap();
    assert!(run(&Call::MoveEtcAside, scratch.path()).status.success());
    assert!(!etc.join("zprofile").exists());
    assert!(!etc.join("zprofile.before-nix-darwin.2").exists());
}

#[test]
fn the_installers_nix_custom_conf_goes_aside_too() {
    let scratch = tempfile::tempdir().unwrap();
    let etc = scratch.path().join("etc");
    fs::create_dir_all(etc.join("nix")).unwrap();
    fs::write(
        etc.join("nix/nix.custom.conf"),
        "# written by the installer\n",
    )
    .unwrap();

    let output = run(&Call::MoveEtcAside, scratch.path());
    assert!(output.status.success(), "{output:?}");
    assert!(!etc.join("nix/nix.custom.conf").exists());
    assert_eq!(
        fs::read_to_string(etc.join("nix/nix.custom.conf.before-nix-darwin")).unwrap(),
        "# written by the installer\n"
    );
}
