//! `tailscale whois --json` answers in the shape tailscale 1.104 writes,
//! trimmed to what is read and with made-up names

use std::collections::BTreeSet;

use crate::web::whois::{Peer, Policy, Verdict};

pub const CAPABILITY: &str = "example.com/cap/coeus";

/// a phone its user signed in, granted nothing
const PHONE: &str = r#"{
  "Node": {
    "ID": 4943427598918140,
    "StableID": "nq9kwhTtbf11CNTRL",
    "Name": "phone.tail0000.ts.net.",
    "User": 5537918502932556,
    "Addresses": ["100.73.0.1/32", "fd7a:115c:a1e0::1/128"],
    "Hostinfo": { "OS": "iOS", "Hostname": "localhost" },
    "Online": true,
    "ComputedName": "phone",
    "ComputedNameWithHost": "phone (localhost)"
  },
  "UserProfile": {
    "ID": 5537918502932556,
    "LoginName": "someone@example.com",
    "DisplayName": "Someone"
  },
  "CapMap": null
}"#;

/// a server owned by a tag
const SERVER: &str = r#"{
  "Node": {
    "ID": 1,
    "Name": "lab.tail0000.ts.net.",
    "User": 6541316759026549,
    "Tags": ["tag:fleet"],
    "ComputedName": "lab"
  },
  "UserProfile": {
    "ID": 6541316759026549,
    "LoginName": "tagged-devices",
    "DisplayName": "Tagged Devices"
  },
  "CapMap": null
}"#;

/// `PHONE` with an app grant: each capability maps to the values every
/// matching grant gave it
fn granted(capability: &str) -> String {
    PHONE.replace(
        r#""CapMap": null"#,
        &format!(
            r#""CapMap": {{ "{capability}": [{{}}], "example.com/cap/other": [{{ "x": 1 }}] }}"#
        ),
    )
}

fn policy(logins: &[&str]) -> Policy {
    Policy {
        capability: CAPABILITY.into(),
        logins: logins.iter().map(|login| (*login).to_owned()).collect(),
    }
}

#[test]
fn whois_names_the_login_and_device() {
    let phone = Peer::parse(PHONE.as_bytes()).unwrap();
    assert_eq!(
        phone,
        Peer {
            login: "someone@example.com".into(),
            device: "phone".into(),
            tagged: false,
            capabilities: BTreeSet::new(),
        }
    );

    let server = Peer::parse(SERVER.as_bytes()).unwrap();
    assert!(server.tagged);
    assert_eq!(server.device, "lab");

    let granted = Peer::parse(granted(CAPABILITY).as_bytes()).unwrap();
    assert_eq!(
        granted.capabilities,
        BTreeSet::from([CAPABILITY.to_owned(), "example.com/cap/other".to_owned()])
    );

    assert!(Peer::parse(b"peer not found").is_err());
    assert!(Peer::parse(PHONE.replace("phone.tail0000", "").as_bytes()).is_err());
}

#[test]
fn a_users_device_signs_in_when_granted_or_listed() {
    let phone = Peer::parse(PHONE.as_bytes()).unwrap();
    let elsewhere = Peer::parse(granted("example.com/cap/another").as_bytes()).unwrap();
    let granted = Peer::parse(granted(CAPABILITY).as_bytes()).unwrap();

    assert_eq!(policy(&[]).judge(&granted), Verdict::Welcome);
    assert_eq!(
        policy(&["someone@example.com"]).judge(&phone),
        Verdict::Welcome
    );
    assert_eq!(policy(&[]).judge(&phone), Verdict::NotGranted);
    assert_eq!(
        policy(&["else@example.com"]).judge(&phone),
        Verdict::NotGranted
    );
    assert_eq!(policy(&[]).judge(&elsewhere), Verdict::NotGranted);
}

#[test]
fn a_tagged_node_signs_in_only_when_the_policy_grants_it() {
    let server = Peer::parse(SERVER.as_bytes()).unwrap();
    let mut granted_server = server.clone();
    granted_server.capabilities.insert(CAPABILITY.into());

    assert_eq!(policy(&[]).judge(&granted_server), Verdict::Welcome);
    assert_eq!(policy(&[]).judge(&server), Verdict::Tagged);
    assert_eq!(policy(&["tagged-devices"]).judge(&server), Verdict::Tagged);
}
