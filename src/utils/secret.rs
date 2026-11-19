//! age files opened in memory with an ssh key, the plaintext zeroed on drop

use std::{io::Read, iter};

use age::armor::ArmoredReader;
use eyre::{Result, eyre};
use zeroize::Zeroizing;

/// an unencrypted OpenSSH ed25519 or rsa private key
pub fn ssh_identity(private: &[u8]) -> Result<age::ssh::Identity> {
    let identity = age::ssh::Identity::from_buffer(private, None)
        .map_err(|error| eyre!("not an ssh private key: {error}"))?;
    match identity {
        age::ssh::Identity::Unencrypted(_) => Ok(identity),
        age::ssh::Identity::Encrypted(_) => Err(eyre!(
            "the key is protected by a passphrase, which coeus does not ask"
        )),
        age::ssh::Identity::Unsupported(_) => Err(eyre!("an ssh-ed25519 or ssh-rsa key is needed")),
    }
}

/// the plaintext of an age file, binary or armored
pub fn decrypt(identity: &dyn age::Identity, ciphertext: &[u8]) -> Result<Zeroizing<Vec<u8>>> {
    let decryptor = age::Decryptor::new(ArmoredReader::new(ciphertext))?;
    let mut reader = decryptor.decrypt(iter::once(identity))?;
    // the plaintext is never longer than the file, so the buffer never
    // grows and leaves no copy behind
    let mut plaintext = Zeroizing::new(Vec::with_capacity(ciphertext.len()));
    reader.read_to_end(&mut plaintext)?;
    Ok(plaintext)
}
