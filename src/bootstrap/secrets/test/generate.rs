use sha_crypt::{PasswordHashRef, PasswordVerifier, ShaCrypt};
use zeroize::Zeroizing;

use crate::bootstrap::{SshKey, contract::Generator, secrets::generate};

#[test]
fn a_password_is_stored_as_a_sha512_crypt_hash() {
    let hash = generate::password("correct horse").unwrap();
    assert!(hash.starts_with("$6$"), "{}", hash.as_str());

    let parsed = PasswordHashRef::new(hash.as_str()).unwrap();
    assert!(
        ShaCrypt::SHA512
            .verify_password(b"correct horse", parsed)
            .is_ok()
    );
    assert!(ShaCrypt::SHA512.verify_password(b"wrong", parsed).is_err());
}

#[test]
fn an_ssh_key_comes_with_its_public_line() {
    let (private, public) = generate::ssh_ed25519("r3dlust@nova").unwrap();
    assert!(private.starts_with("-----BEGIN OPENSSH PRIVATE KEY-----\n"));
    assert!(public.ends_with(" r3dlust@nova"), "{public}");
    assert!(SshKey::parse(&public).is_ok());
}

#[test]
fn a_random_secret_is_lowercase_hex() {
    let secret = generate::random(32);
    assert_eq!(secret.len(), 64);
    assert!(
        secret
            .iter()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
    );
    assert_ne!(*secret, *generate::random(32));
}

#[test]
fn each_generator_makes_its_kind_of_secret() {
    let typed = || Some(Zeroizing::new("hunter2".to_owned()));

    let random = generate::generate(&Generator::Random { bytes: 16 }, None).unwrap();
    assert_eq!(random.plaintext.len(), 32);
    assert!(random.public.is_none());

    let text = generate::generate(&Generator::Text { prompt: None }, typed()).unwrap();
    assert_eq!(&*text.plaintext, b"hunter2");

    let password = generate::generate(&Generator::Password { prompt: None }, typed()).unwrap();
    assert!(password.plaintext.starts_with(b"$6$"));

    let ssh = generate::generate(
        &Generator::SshEd25519 {
            comment: Some("deploy@nova".to_owned()),
        },
        None,
    )
    .unwrap();
    assert!(
        ssh.plaintext
            .starts_with(b"-----BEGIN OPENSSH PRIVATE KEY-----")
    );
    assert!(ssh.public.unwrap().ends_with(" deploy@nova"));
}

#[test]
fn a_typed_secret_needs_what_was_typed() {
    assert!(generate::generate(&Generator::Text { prompt: None }, None).is_err());
    assert!(generate::generate(&Generator::Password { prompt: None }, None).is_err());
}
