use std::fs;

use tempfile::tempdir;

use super::{keypair, seal, write_identity};
use crate::bootstrap::secrets::AdminIdentity;

#[tokio::test]
async fn a_readable_openssh_key_opens_what_names_it() {
    let directory = tempdir().unwrap();
    let path = write_identity(directory.path(), "altar");
    let public = AdminIdentity::public(&path).unwrap();
    let identity = AdminIdentity::unlock(&path).await.unwrap();
    assert_eq!(identity.key(), &public);

    let (_, other) = keypair("other");
    for armored in [false, true] {
        let file = seal(b"the plaintext", &[&other, &public], &[], armored);
        assert_eq!(&*identity.decrypt(&file).unwrap(), b"the plaintext");
    }
    let elsewhere = seal(b"the plaintext", &[&other], &[], false);
    assert!(identity.decrypt(&elsewhere).is_err());
}

#[tokio::test]
async fn a_private_key_that_is_not_its_pub_files_is_refused() {
    let directory = tempdir().unwrap();
    let path = write_identity(directory.path(), "altar");
    let (_, other) = keypair("other");
    fs::write(
        directory.path().join("altar.pub"),
        format!("{other} root@other\n"),
    )
    .unwrap();

    let error = AdminIdentity::unlock(&path).await.err().unwrap();
    assert!(
        format!("{error:#}").contains("not the one its .pub file names"),
        "{error:#}"
    );
}
