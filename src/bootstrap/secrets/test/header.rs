use super::{keypair, seal};
use crate::bootstrap::secrets::{Header, Stanza, Verdict};

fn tags(header: &Header) -> Vec<&str> {
    header
        .stanzas
        .iter()
        .filter_map(|stanza| match stanza {
            Stanza::Ssh { tag } => Some(tag.as_str()),
            _ => None,
        })
        .collect()
}

#[test]
fn a_header_names_each_ssh_recipient_by_its_tag() {
    let keys: Vec<_> = (0..3).map(|n| keypair(&format!("host{n}")).1).collect();
    for count in 1..=3 {
        let wanted: Vec<_> = keys[..count].iter().collect();
        for armored in [false, true] {
            let file = seal(b"secret", &wanted, &[], armored);
            let header = Header::from_reader(&file[..]).unwrap();

            assert_eq!(header.armored, armored);
            let expected: Vec<String> = wanted.iter().map(|key| key.age_tag()).collect();
            assert_eq!(tags(&header), expected);
            // age adds one grease stanza to every file it writes
            assert!(header.stanzas.contains(&Stanza::Other), "{header:?}");
        }
    }
}

#[test]
fn only_the_header_is_read() {
    let (_, key) = keypair("host");
    let file = seal(b"secret", &[&key], &[], false);
    let end = file
        .windows(4)
        .position(|window| window == b"\n---")
        .unwrap();
    let line_end = end
        + 1
        + file[end + 1..]
            .iter()
            .position(|&byte| byte == b'\n')
            .unwrap();

    let mut cut = file[..=line_end].to_vec();
    cut.extend_from_slice(b"\xff\xfe not a payload");
    let header = Header::from_reader(&cut[..]).unwrap();
    assert_eq!(tags(&header), [key.age_tag()]);
}

#[test]
fn equal_sets_of_keys_are_current_whatever_their_order() {
    let (_, a) = keypair("a");
    let (_, b) = keypair("b");
    let header = Header::from_reader(&seal(b"secret", &[&a, &b], &[], true)[..]).unwrap();

    assert_eq!(header.verdict(&[b.clone(), a.clone()]), Verdict::Current);
    assert_eq!(
        header.verdict(&[a.clone(), b.clone(), a.clone()]),
        Verdict::Current
    );
}

#[test]
fn a_different_set_is_stale_by_what_it_lacks_and_holds_beyond() {
    let (_, a) = keypair("a");
    let (_, b) = keypair("b");
    let (_, c) = keypair("c");
    let header = Header::from_reader(&seal(b"secret", &[&a, &b], &[], false)[..]).unwrap();

    assert_eq!(
        header.verdict(&[a.clone(), b.clone(), c.clone()]),
        Verdict::Stale {
            missing: 1,
            extra: 0
        }
    );
    assert_eq!(
        header.verdict(std::slice::from_ref(&a)),
        Verdict::Stale {
            missing: 0,
            extra: 1
        }
    );
    assert_eq!(
        header.verdict(std::slice::from_ref(&c)),
        Verdict::Stale {
            missing: 1,
            extra: 2
        }
    );
}

#[test]
fn an_x25519_recipient_is_one_the_contract_does_not_name() {
    let (_, a) = keypair("a");
    let x25519 = age::x25519::Identity::generate().to_public();
    let header = Header::from_reader(&seal(b"secret", &[&a], &[x25519], false)[..]).unwrap();

    assert_eq!(
        header
            .stanzas
            .iter()
            .filter(|s| **s == Stanza::X25519)
            .count(),
        1
    );
    assert_eq!(
        header.verdict(std::slice::from_ref(&a)),
        Verdict::Stale {
            missing: 0,
            extra: 1
        }
    );
}

#[test]
fn names_tells_whether_a_key_opens_the_file() {
    let (_, a) = keypair("a");
    let (_, b) = keypair("b");
    let header = Header::from_reader(&seal(b"secret", &[&a], &[], false)[..]).unwrap();
    assert!(header.names(&a));
    assert!(!header.names(&b));
}

#[test]
fn what_is_not_an_age_header_is_refused() {
    assert!(Header::from_reader(&b"hello\n"[..]).is_err());
    assert!(Header::from_reader(&b""[..]).is_err());

    let (_, a) = keypair("a");
    let file = seal(b"secret", &[&a], &[], false);
    let end = file
        .windows(4)
        .position(|window| window == b"\n---")
        .unwrap();
    assert!(Header::from_reader(&file[..end]).is_err());
}
