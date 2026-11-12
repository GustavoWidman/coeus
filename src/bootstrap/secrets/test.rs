use sha_crypt::{PasswordHashRef, PasswordVerifier, ShaCrypt};

use super::generate;
use crate::bootstrap::SshKey;

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
