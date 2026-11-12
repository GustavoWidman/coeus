//! age files made by the age crate, for keys made on the spot

mod generate;
mod header;
mod identity;

use std::{io::Write, path::Path};

use age::armor::{ArmoredWriter, Format};

use super::{AdminIdentity, generate::ssh_ed25519};
use crate::bootstrap::SshKey;

/// a new ed25519 key: its OpenSSH private key and its public key
fn keypair(comment: &str) -> (String, SshKey) {
    let (private, public) = ssh_ed25519(comment).unwrap();
    (private.to_string(), SshKey::parse(&public).unwrap())
}

/// `plaintext` encrypted the way `age -R` would, to ssh keys and any
/// X25519 recipients
fn seal(
    plaintext: &[u8],
    keys: &[&SshKey],
    x25519: &[age::x25519::Recipient],
    armored: bool,
) -> Vec<u8> {
    let ssh: Vec<age::ssh::Recipient> = keys
        .iter()
        .map(|key| key.to_string().parse().unwrap())
        .collect();
    let recipients = ssh
        .iter()
        .map(|recipient| recipient as &dyn age::Recipient)
        .chain(
            x25519
                .iter()
                .map(|recipient| recipient as &dyn age::Recipient),
        );
    let encryptor = age::Encryptor::with_recipients(recipients).unwrap();

    let format = if armored {
        Format::AsciiArmor
    } else {
        Format::Binary
    };
    let mut file = Vec::new();
    let mut writer = encryptor
        .wrap_output(ArmoredWriter::wrap_output(&mut file, format).unwrap())
        .unwrap();
    writer.write_all(plaintext).unwrap();
    writer.finish().unwrap().finish().unwrap();
    file
}

/// an admin whose identity is a readable OpenSSH key file in `directory`
async fn admin(directory: &Path, name: &str) -> AdminIdentity {
    let path = write_identity(directory, name);
    AdminIdentity::unlock(&path).await.unwrap()
}

/// writes `<name>` and `<name>.pub` and returns the private key's path
fn write_identity(directory: &Path, name: &str) -> std::path::PathBuf {
    let (private, public) = ssh_ed25519(&format!("root@{name}")).unwrap();
    let path = directory.join(name);
    std::fs::write(&path, private.as_bytes()).unwrap();
    std::fs::write(directory.join(format!("{name}.pub")), format!("{public}\n")).unwrap();
    path
}
