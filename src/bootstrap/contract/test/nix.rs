//! the contract evaluated by a real nix; skipped where there is none

use std::{
    fs,
    path::{Path, PathBuf},
};

use crate::bootstrap::{
    SshKey,
    contract::{
        Answers, Contract, DEFAULT_PATH, Failure, Generator, Input, QuestionKind, RepoPath,
        nix_string,
    },
    remote::Facts,
};

const VM: &str = include_str!("fixtures/input-vm.json");
const MAC: &str = include_str!("fixtures/input-mac.json");

fn has_nix() -> bool {
    let found = std::process::Command::new("nix")
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success());
    if !found {
        eprintln!("skipped: no nix on PATH");
    }
    found
}

fn repo(contract: &str) -> tempfile::TempDir {
    let repo = tempfile::tempdir().unwrap();
    let file = repo.path().join(DEFAULT_PATH);
    fs::create_dir_all(file.parent().unwrap()).unwrap();
    fs::write(file, contract).unwrap();
    repo
}

async fn open(repo: &Path) -> eyre::Result<Contract> {
    Contract::open(repo, Path::new(DEFAULT_PATH)).await
}

fn input(sample: &str, answers: &[(&str, &str)]) -> Input {
    let value: serde_json::Value = serde_json::from_str(sample).unwrap();
    let mut given = Answers::default();
    for (id, answer) in answers {
        given.insert((*id).into(), (*answer).into());
    }
    Input {
        name: value["name"].as_str().unwrap().parse().unwrap(),
        host_key: SshKey::parse(value["hostKey"].as_str().unwrap()).unwrap(),
        facts: serde_json::from_value::<Facts>(value["facts"].clone()).unwrap(),
        answers: given,
    }
}

fn path(path: &str) -> RepoPath {
    path.parse().unwrap()
}

#[tokio::test]
async fn any_text_reaches_nix_as_it_is() {
    if !has_nix() {
        return;
    }
    let repo = repo("{ version = 1; identity = \"/id\"; echo = x: x; }");
    let contract = open(repo.path()).await.unwrap();

    let text = "a \"quote\", a \\, ${builtins.abort \"no\"}, $${x}, $\r\n\ttab, ünï, end$";
    let echoed: String =
        super::super::json(&contract.file, &format!("c: c.echo {}", nix_string(text)))
            .await
            .unwrap();
    assert_eq!(echoed, text);
}

#[tokio::test]
async fn a_contract_that_throws_is_heard_in_its_own_words() {
    if !has_nix() {
        return;
    }
    let repo = repo(
        r#"{
          version = 1;
          identity = "/etc/ssh/ssh_host_ed25519_key";
          questions = input: throw "coeus bootstrap: ${input.name} is not a name this fleet takes";
          enroll = input: abort "nothing to enroll";
          recipients = path: if path == "broken.age" then { }.missing else null;
        }"#,
    );
    let contract = open(repo.path()).await.unwrap();
    let nova = input(VM, &[]);

    let error = contract.questions(&nova).await.unwrap_err();
    assert_eq!(
        error.downcast_ref::<Failure>(),
        Some(&Failure::Said(
            "coeus bootstrap: vm-sample is not a name this fleet takes".into()
        ))
    );
    assert_eq!(
        error.to_string(),
        "coeus bootstrap: vm-sample is not a name this fleet takes"
    );

    let error = contract.enroll(&nova).await.unwrap_err();
    assert_eq!(error.to_string(), "nothing to enroll");

    let error = contract
        .recipients(&[path("broken.age")])
        .await
        .unwrap_err();
    let Some(Failure::Broken {
        error,
        at: Some(at),
    }) = error.downcast_ref::<Failure>()
    else {
        panic!("{error:#}");
    };
    assert_eq!(error, "attribute 'missing' missing");
    assert!(at.contains("bootstrap.nix:6:"), "{at}");
}

#[tokio::test]
async fn a_contract_of_another_version_or_none_is_refused() {
    if !has_nix() {
        return;
    }
    let newer = repo("{ version = 2; identity = \"/id\"; }");
    let error = open(newer.path()).await.err().unwrap();
    assert!(error.to_string().contains("update coeus"), "{error}");

    let none = tempfile::tempdir().unwrap();
    let error = open(none.path()).await.err().unwrap();
    assert!(error.to_string().contains(DEFAULT_PATH), "{error}");
}

/// a copy of what the fleet repository's contract reads: the contract,
/// `keys.nix`, and each machine's `flake.nix` and `host.pub`
fn fleet() -> Option<tempfile::TempDir> {
    let Some(source) = std::env::var_os("COEUS_FLEET_REPO").map(PathBuf::from) else {
        eprintln!("skipped: COEUS_FLEET_REPO names no checkout of the fleet repository");
        return None;
    };
    if !has_nix() {
        return None;
    }
    let copy = repo(&fs::read_to_string(source.join(DEFAULT_PATH)).unwrap());
    fs::copy(source.join("keys.nix"), copy.path().join("keys.nix")).unwrap();
    for host in fs::read_dir(source.join("hosts")).unwrap() {
        let host = host.unwrap().path();
        let to = copy.path().join("hosts").join(host.file_name().unwrap());
        fs::create_dir_all(&to).unwrap();
        for file in ["flake.nix", "host.pub"] {
            if host.join(file).exists() {
                fs::copy(host.join(file), to.join(file)).unwrap();
            }
        }
    }
    Some(copy)
}

#[tokio::test]
async fn the_fleet_contract_enrolls_a_nixos_machine() {
    let Some(repo) = fleet() else { return };
    let contract = open(repo.path()).await.unwrap();
    assert_eq!(
        contract.identity(),
        Path::new("/etc/ssh/ssh_host_ed25519_key")
    );

    let questions = contract.questions(&input(VM, &[])).await.unwrap();
    let ids: Vec<&str> = questions.iter().map(|q| q.id.as_str()).collect();
    assert_eq!(ids, ["type", "admin"]);
    let QuestionKind::Choice { default, .. } = &questions[0].kind else {
        panic!("{:?}", questions[0]);
    };
    assert_eq!(default.as_deref(), Some("server"));
    assert_eq!(questions[1].accept(""), Ok("false".into()));

    let vm = input(VM, &[("type", "server"), ("admin", "false")]);
    let enrollment = contract.enroll(&vm).await.unwrap();
    let files: Vec<&str> = enrollment.files.keys().map(RepoPath::as_str).collect();
    assert_eq!(
        files,
        [
            "hosts/vm-sample/boot.nix",
            "hosts/vm-sample/flake.nix",
            "hosts/vm-sample/hardware.nix",
            "hosts/vm-sample/host.pub",
        ]
    );
    assert_eq!(
        enrollment.files[&path("hosts/vm-sample/host.pub")],
        format!("{}\n", vm.host_key)
    );
    assert_eq!(
        Some(&enrollment.files[&path("hosts/vm-sample/hardware.nix")]),
        vm.facts.hardware_config.as_ref()
    );
    assert_eq!(
        enrollment.secrets,
        [
            (
                path("hosts/vm-sample/password.age"),
                Generator::Password {
                    prompt: Some("password for root and r3dlust on vm-sample".into())
                }
            ),
            (
                path("modules/common/ssh/main/vm-sample.age"),
                Generator::SshEd25519 {
                    comment: Some("r3dlust@vm-sample".into())
                }
            ),
        ]
        .into()
    );
    assert!(enrollment.first_switch.install_bootloader);
    assert!(enrollment.messages.body.ends_with('\n'));

    // the new machine's own secret has no recipients until its files exist
    let own = path("hosts/vm-sample/password.age");
    let error = contract
        .recipients(std::slice::from_ref(&own))
        .await
        .unwrap_err();
    assert!(
        matches!(error.downcast_ref::<Failure>(), Some(Failure::Said(said)) if said.contains("hosts/vm-sample")),
        "{error:#}"
    );

    for (file, text) in &enrollment.files {
        let at = repo.path().join(file.as_str());
        fs::create_dir_all(at.parent().unwrap()).unwrap();
        fs::write(at, text).unwrap();
    }
    let shared = path("modules/common/ssh/main/vm-sample.age");
    let unmanaged = path("hosts/oracle-1/niks3/api-token.age");
    let plain = path("hosts/vm-sample/flake.nix");
    let rules = contract
        .recipients(&[
            own.clone(),
            shared.clone(),
            unmanaged.clone(),
            plain.clone(),
        ])
        .await
        .unwrap();

    let own = rules[&own].as_ref().unwrap();
    let shared = rules[&shared].as_ref().unwrap();
    assert_eq!(own[0], vm.host_key);
    assert!(own.len() > 1, "the admins open it too");
    assert!(shared.contains(&vm.host_key));
    assert!(own.iter().all(|key| shared.contains(key)));
    assert_eq!(rules[&unmanaged], None);
    assert_eq!(rules[&plain], None);
}

#[tokio::test]
async fn the_fleet_contract_enrolls_a_mac_without_a_password() {
    let Some(repo) = fleet() else { return };
    let contract = open(repo.path()).await.unwrap();

    let questions = contract.questions(&input(MAC, &[])).await.unwrap();
    assert_eq!(questions[0].accept(""), Ok("desktop".into()));

    let mac = input(MAC, &[("type", "desktop"), ("admin", "true")]);
    let enrollment = contract.enroll(&mac).await.unwrap();
    let files: Vec<&str> = enrollment.files.keys().map(RepoPath::as_str).collect();
    assert_eq!(
        files,
        ["hosts/mac-sample/flake.nix", "hosts/mac-sample/host.pub"]
    );
    assert!(enrollment.files[&path("hosts/mac-sample/flake.nix")].contains("admin = true;"));
    let secrets: Vec<&str> = enrollment.secrets.keys().map(RepoPath::as_str).collect();
    assert_eq!(secrets, ["modules/common/ssh/main/mac-sample.age"]);
    assert!(!enrollment.first_switch.install_bootloader);

    for (file, text) in &enrollment.files {
        let at = repo.path().join(file.as_str());
        fs::create_dir_all(at.parent().unwrap()).unwrap();
        fs::write(at, text).unwrap();
    }
    // an admin opens a secret of every other machine
    let rules = contract
        .recipients(&[path("hosts/oracle-1/password.age")])
        .await
        .unwrap();
    assert!(
        rules[&path("hosts/oracle-1/password.age")]
            .as_ref()
            .unwrap()
            .contains(&mac.host_key)
    );
}
