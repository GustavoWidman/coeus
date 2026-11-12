use std::os::unix::fs::PermissionsExt;

use crate::bootstrap::{SshKey, remote::SessionKey, state::StateDir};

#[test]
fn the_session_key_is_made_once_and_kept_private() {
    let home = tempfile::tempdir().unwrap();
    let state = StateDir::under(home.path(), &"nova".parse().unwrap());
    assert!(SessionKey::existing(&state).unwrap().is_none());

    let made = SessionKey::ensure(&state).unwrap();
    let mode = std::fs::metadata(made.private())
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o600);

    let public = std::fs::read_to_string(state.session_key().with_extension("pub")).unwrap();
    assert!(
        public.trim_end().ends_with(" coeus-bootstrap@nova"),
        "{public}"
    );
    assert_eq!(&SshKey::parse(&public).unwrap(), made.public());

    let private = std::fs::read_to_string(made.private()).unwrap();
    assert!(private.starts_with("-----BEGIN OPENSSH PRIVATE KEY-----"));

    let again = SessionKey::ensure(&state).unwrap();
    assert_eq!(again.public(), made.public());
    assert_eq!(
        SessionKey::existing(&state).unwrap().unwrap().public(),
        made.public()
    );
}
