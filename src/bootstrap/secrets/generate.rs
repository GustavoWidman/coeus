//! a new secret's plaintext, from one of the contract's generators. what the
//! person types is asked by the run and handed in; nothing here prompts.

use eyre::eyre;
use sha_crypt::{PasswordHasher, ShaCrypt};
use ssh_key::{Algorithm, LineEnding, PrivateKey, rand_core::OsRng};
use zeroize::Zeroizing;

use crate::bootstrap::contract::Generator;

pub struct Generated {
    pub plaintext: Zeroizing<Vec<u8>>,
    /// an ssh key's public line, `ssh-ed25519 AAAA… <comment>`, for the
    /// pull request
    pub public: Option<String>,
}

/// makes the secret; `typed` is what the person typed for a password or a
/// text, and must be there for those
pub fn generate(
    generator: &Generator,
    typed: Option<Zeroizing<String>>,
) -> eyre::Result<Generated> {
    todo!()
}

/// `$6$<salt>$<hash>`, sha512-crypt with a random salt
pub fn password(typed: &str) -> eyre::Result<Zeroizing<String>> {
    let hash = ShaCrypt::SHA512
        .hash_password(typed.as_bytes())
        .map_err(|error| eyre!("failed to hash the password: {error}"))?;
    Ok(Zeroizing::new(hash.as_str().to_owned()))
}

/// an OpenSSH private key and its public line
pub fn ssh_ed25519(comment: &str) -> eyre::Result<(Zeroizing<String>, String)> {
    let mut key = PrivateKey::random(&mut OsRng, Algorithm::Ed25519)?;
    key.set_comment(comment);
    Ok((
        key.to_openssh(LineEnding::LF)?,
        key.public_key().to_openssh()?,
    ))
}

/// `bytes` random bytes as lowercase hex
pub fn random(bytes: usize) -> Zeroizing<Vec<u8>> {
    todo!()
}
