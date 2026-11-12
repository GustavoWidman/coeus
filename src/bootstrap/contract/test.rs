use serde_json::json;

use super::{Answers, Input, RepoPath, wire};
use crate::bootstrap::{SshKey, remote::test::nixos};

const NOVA: &str =
    "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIBqyOKVkbPDCemEVQJa2aQiocqctG5AenH2AeYZEHd/L";

#[test]
fn the_input_is_name_host_key_facts_and_answers() {
    let mut answers = Answers::default();
    answers.insert("type".into(), "server".into());
    let input = Input {
        name: "nova".parse().unwrap(),
        host_key: SshKey::parse(NOVA).unwrap(),
        facts: nixos(),
        answers,
    };

    let value = serde_json::to_value(&input).unwrap();
    let keys: Vec<&str> = value
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(keys, ["answers", "facts", "hostKey", "name"]);
    assert_eq!(value["name"], "nova");
    assert_eq!(value["hostKey"], NOVA);
    assert_eq!(value["answers"], json!({ "type": "server" }));
    assert_eq!(value["facts"]["osVersion"], "26.05");
}

#[test]
fn questions_of_each_kind_parse() {
    let questions: Vec<wire::Question> = serde_json::from_value(json!([
        {
            "id": "type", "prompt": "what kind of machine is it?", "kind": "choice",
            "choices": [
                { "value": "server", "label": "server", "help": "always on" },
                { "value": "desktop", "label": "desktop" },
            ],
            "default": "server",
        },
        { "id": "admin", "prompt": "an admin?", "kind": "confirm", "default": false },
        { "id": "zone", "prompt": "time zone", "kind": "text", "default": null, "pattern": "[A-Za-z_/]+" },
    ]))
    .unwrap();

    assert_eq!(questions[0].kind, wire::QuestionKind::Choice);
    assert_eq!(questions[0].choices.as_ref().unwrap()[1].help, None);
    assert_eq!(
        questions[0].default,
        Some(wire::Default::Text("server".into()))
    );
    assert_eq!(questions[1].default, Some(wire::Default::Bool(false)));
    assert_eq!(questions[2].default, None);
    assert_eq!(questions[2].pattern.as_deref(), Some("[A-Za-z_/]+"));
}

#[test]
fn a_question_the_schema_does_not_describe_is_refused() {
    for bad in [
        json!({ "id": "a", "prompt": "p", "kind": "text", "hint": "?" }),
        json!({ "id": "a", "prompt": "p", "kind": "number" }),
        json!({ "id": "a", "kind": "text" }),
        json!({ "id": "a", "prompt": "p", "kind": "choice", "choices": [{ "value": "x" }] }),
    ] {
        assert!(
            serde_json::from_value::<wire::Question>(bad.clone()).is_err(),
            "{bad}"
        );
    }
}

fn enrollment() -> serde_json::Value {
    json!({
        "files": { "hosts/nova/flake.nix": "{ }\n", "hosts/nova/host.pub": format!("{NOVA}\n") },
        "secrets": {
            "hosts/nova/password.age": { "kind": "password", "prompt": "root's password" },
            "modules/common/ssh/main/nova.age": { "kind": "ssh-ed25519", "comment": "r3dlust@nova" },
            "hosts/nova/token.age": { "kind": "random" },
            "hosts/nova/api.age": { "kind": "text", "prompt": "paste the token", "comment": null, "bytes": 16 },
        },
        "firstSwitch": { "installBootloader": true },
        "messages": {
            "enroll": "feat(nova): join the fleet",
            "rekey": "chore(nova): rekey secrets",
            "title": "feat(nova): join the fleet",
            "body": "adds nova",
        },
        "notes": ["sign nova in to Tailscale"],
    })
}

#[test]
fn an_enrollment_parses_with_its_defaults() {
    let enrollment: wire::Enrollment = serde_json::from_value(enrollment()).unwrap();

    assert_eq!(enrollment.files.len(), 2);
    assert!(enrollment.first_switch.install_bootloader);
    assert_eq!(enrollment.messages.rekey, "chore(nova): rekey secrets");
    assert_eq!(enrollment.notes, ["sign nova in to Tailscale"]);

    let token = &enrollment.secrets["hosts/nova/token.age"];
    assert_eq!(token.kind, wire::SecretKind::Random);
    assert_eq!(token.bytes, 32);
    assert_eq!(enrollment.secrets["hosts/nova/api.age"].bytes, 16);
    assert_eq!(
        enrollment.secrets["modules/common/ssh/main/nova.age"].kind,
        wire::SecretKind::SshEd25519
    );
}

#[test]
fn an_enrollment_missing_a_part_or_with_an_extra_one_is_refused() {
    for field in ["files", "secrets", "firstSwitch", "messages", "notes"] {
        let mut value = enrollment();
        value.as_object_mut().unwrap().remove(field);
        assert!(
            serde_json::from_value::<wire::Enrollment>(value).is_err(),
            "{field}"
        );
    }

    let mut extra = enrollment();
    extra["firstSwitch"]["action"] = json!("boot");
    assert!(serde_json::from_value::<wire::Enrollment>(extra).is_err());

    let mut kind = enrollment();
    kind["secrets"]["hosts/nova/token.age"]["kind"] = json!("command");
    assert!(serde_json::from_value::<wire::Enrollment>(kind).is_err());
}

#[test]
fn the_head_and_the_recipients_parse() {
    let head: wire::Head = serde_json::from_value(
        json!({ "version": 1, "identity": "/etc/ssh/ssh_host_ed25519_key" }),
    )
    .unwrap();
    assert_eq!(head.version, 1);

    let recipients: wire::Recipients = serde_json::from_value(json!({
        "hosts/nova/password.age": [NOVA],
        "hosts/oracle-1/niks3/api-token.age": null,
    }))
    .unwrap();
    assert_eq!(recipients["hosts/oracle-1/niks3/api-token.age"], None);
}

#[test]
fn repository_paths_stay_inside_the_repository() {
    for good in [
        "hosts/nova/flake.nix",
        "a.age",
        ".coeus/bootstrap.nix",
        "x/.gitignore",
    ] {
        assert_eq!(good.parse::<RepoPath>().unwrap().as_str(), good);
    }
    for bad in [
        "",
        "/etc/passwd",
        "../x",
        "hosts/../../x",
        "hosts//nova",
        "./hosts",
        "hosts/nova/",
        ".git/config",
        "x/.git/hooks",
        "a\\b",
    ] {
        assert!(bad.parse::<RepoPath>().is_err(), "{bad:?}");
    }
    assert!(
        "hosts/nova/password.age"
            .parse::<RepoPath>()
            .unwrap()
            .is_secret()
    );
}
