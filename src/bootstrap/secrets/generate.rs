//! a new secret's plaintext, from one of the contract's generators. what the
//! person types is asked by the run and handed in; nothing here prompts.

use eyre::{bail, eyre};
use rand::Rng;
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
    let typed =
        || typed.ok_or_else(|| eyre!("this secret is typed by a person, and nothing was typed"));
    let (plaintext, public) = match generator {
        Generator::Password { .. } => (into_bytes(password(&typed()?)?), None),
        Generator::SshEd25519 { comment } => {
            let (private, public) = ssh_ed25519(comment.as_deref().unwrap_or_default())?;
            (into_bytes(private), Some(public))
        }
        Generator::Random { bytes: 0 } => {
            bail!("a random secret of no bytes is no secret");
        }
        Generator::Random { bytes } => (random(*bytes), None),
        Generator::Text { .. } => (into_bytes(typed()?), None),
    };
    Ok(Generated { plaintext, public })
}

/// the string's bytes, moved rather than copied so no copy is left unzeroed
fn into_bytes(mut text: Zeroizing<String>) -> Zeroizing<Vec<u8>> {
    Zeroizing::new(std::mem::take(&mut *text).into_bytes())
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
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut raw = Zeroizing::new(vec![0; bytes]);
    rand::rng().fill_bytes(&mut raw);
    let mut hex = Zeroizing::new(Vec::with_capacity(bytes * 2));
    for byte in raw.iter() {
        hex.extend([
            DIGITS[usize::from(byte >> 4)],
            DIGITS[usize::from(byte & 0xf)],
        ]);
    }
    hex
}
