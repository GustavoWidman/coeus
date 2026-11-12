use serde_json::json;

use super::enrollment as sample;
use crate::bootstrap::contract::{Enrollment, Generator, RepoPath, wire};

fn enroll(value: serde_json::Value) -> eyre::Result<Enrollment> {
    serde_json::from_value::<wire::Enrollment>(value)
        .unwrap()
        .try_into()
}

fn path(path: &str) -> RepoPath {
    path.parse().unwrap()
}

#[test]
fn every_secret_becomes_its_generator() {
    let enrollment = enroll(sample()).unwrap();

    assert_eq!(
        enrollment.files[&path("hosts/nova/flake.nix")],
        "{ }\n".to_owned()
    );
    assert_eq!(
        enrollment.secrets[&path("hosts/nova/password.age")],
        Generator::Password {
            prompt: Some("root's password".into())
        }
    );
    assert_eq!(
        enrollment.secrets[&path("modules/common/ssh/main/nova.age")],
        Generator::SshEd25519 {
            comment: Some("r3dlust@nova".into())
        }
    );
    assert_eq!(
        enrollment.secrets[&path("hosts/nova/token.age")],
        Generator::Random { bytes: 32 }
    );
    assert_eq!(
        enrollment.secrets[&path("hosts/nova/api.age")],
        Generator::Text {
            prompt: Some("paste the token".into())
        }
    );
    assert!(enrollment.first_switch.install_bootloader);
    assert_eq!(enrollment.messages.enroll, "feat(nova): join the fleet");
    assert_eq!(enrollment.notes, ["sign nova in to Tailscale"]);
}

#[test]
fn a_path_outside_the_repository_or_in_the_wrong_place_is_refused() {
    let cases = [
        ("files", "../etc/passwd", json!("x")),
        ("files", "/etc/passwd", json!("x")),
        ("files", ".git/hooks/post-checkout", json!("x")),
        ("files", "hosts/nova/password.age", json!("x")),
        (
            "secrets",
            "hosts/nova/../../x.age",
            json!({ "kind": "random" }),
        ),
        (
            "secrets",
            "hosts/nova/password",
            json!({ "kind": "password" }),
        ),
    ];
    for (part, at, value) in cases {
        let mut enrollment = sample();
        enrollment[part][at] = value;
        assert!(enroll(enrollment).is_err(), "{part} {at}");
    }
}

#[test]
fn a_secret_with_a_field_its_kind_does_not_use_is_refused() {
    for (at, secret) in [
        (
            "hosts/nova/a.age",
            json!({ "kind": "password", "comment": "x" }),
        ),
        (
            "hosts/nova/a.age",
            json!({ "kind": "ssh-ed25519", "prompt": "x" }),
        ),
        (
            "hosts/nova/a.age",
            json!({ "kind": "random", "prompt": "x" }),
        ),
        (
            "hosts/nova/a.age",
            json!({ "kind": "random", "comment": "x" }),
        ),
        ("hosts/nova/a.age", json!({ "kind": "random", "bytes": 0 })),
        (
            "hosts/nova/a.age",
            json!({ "kind": "text", "comment": "x" }),
        ),
    ] {
        let mut enrollment = sample();
        enrollment["secrets"][at] = secret.clone();
        let error = enroll(enrollment).unwrap_err();
        assert!(format!("{error:#}").contains(at), "{secret}: {error:#}");
    }
}
