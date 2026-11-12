use std::io::Write;

use crate::bootstrap::{SshKey, key::KeyKind};

/// made with `ssh-keygen -t ed25519 -C root@nova`
const NOVA: &str =
    "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIBqyOKVkbPDCemEVQJa2aQiocqctG5AenH2AeYZEHd/L root@nova";
const NOVA_BARE: &str =
    "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIBqyOKVkbPDCemEVQJa2aQiocqctG5AenH2AeYZEHd/L";
const ALTAR: &str =
    "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIN9FLpE3q6S85tUkrfN6U7VMxHTkk6ikkPnNkS9uU7qx altar";

#[test]
fn a_key_drops_its_comment() {
    let key = SshKey::parse(NOVA).unwrap();
    assert_eq!(key.kind(), KeyKind::Ed25519);
    assert_eq!(key.to_string(), NOVA_BARE);
    assert_eq!(key, SshKey::parse(NOVA_BARE).unwrap());
    assert_eq!(key, SshKey::parse(&format!("  {NOVA}\n")).unwrap());
}

#[test]
fn the_fingerprint_is_what_ssh_keygen_prints() {
    assert_eq!(
        SshKey::parse(NOVA).unwrap().fingerprint(),
        "SHA256:aGHFy3oz4S3mttcCbf4q3OEN0qicHbQVvJvtBro9GEQ"
    );
}

#[test]
fn the_age_tag_is_what_age_writes_in_a_stanza() {
    let nova = SshKey::parse(NOVA).unwrap();
    let altar = SshKey::parse(ALTAR).unwrap();
    assert_eq!(nova.age_tag(), "aGHFyw");
    assert_eq!(altar.age_tag(), "QcTekQ");

    let recipients: Vec<age::ssh::Recipient> = [NOVA, ALTAR]
        .iter()
        .map(|key| key.parse().unwrap())
        .collect();
    let encryptor = age::Encryptor::with_recipients(
        recipients
            .iter()
            .map(|recipient| recipient as &dyn age::Recipient),
    )
    .unwrap();
    let mut file = Vec::new();
    let mut writer = encryptor.wrap_output(&mut file).unwrap();
    writer.write_all(b"secret").unwrap();
    writer.finish().unwrap();

    let header = String::from_utf8_lossy(&file);
    let tags: Vec<&str> = header
        .lines()
        .take_while(|line| !line.starts_with("---"))
        .filter_map(|line| line.strip_prefix("-> ssh-ed25519 "))
        .map(|args| args.split(' ').next().unwrap())
        .collect();
    assert_eq!(tags, [nova.age_tag(), altar.age_tag()]);
}

#[test]
fn what_is_not_an_ed25519_or_rsa_key_is_refused() {
    let blob = NOVA_BARE.split(' ').nth(1).unwrap();
    for bad in [
        "",
        "ssh-ed25519",
        "ssh-ed25519 not-base64!",
        &format!("ecdsa-sha2-nistp256 {blob}"),
        // the text says rsa, the blob says ed25519
        &format!("ssh-rsa {blob}"),
        // cut short
        &format!("ssh-ed25519 {}", &blob[..40]),
    ] {
        assert!(SshKey::parse(bad).is_err(), "{bad:?}");
    }
}

#[test]
fn a_key_is_a_string_in_json() {
    let key = SshKey::parse(NOVA).unwrap();
    let json = serde_json::to_string(&key).unwrap();
    assert_eq!(json, format!("\"{NOVA_BARE}\""));
    assert_eq!(serde_json::from_str::<SshKey>(&json).unwrap(), key);
}
