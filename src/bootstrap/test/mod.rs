mod key;
mod run;
mod state;
mod step;

#[test]
fn a_peer_is_found_by_its_magicdns_name() {
    let status = br#"{"MagicDNSSuffix":"tail1.ts.net","Peer":{
        "nodekey:a":{"HostName":"localhost","DNSName":"nova.tail1.ts.net.","TailscaleIPs":["100.99.40.45","fd7a::1"]},
        "nodekey:b":{"HostName":"nova","DNSName":"nova-1.tail1.ts.net.","TailscaleIPs":["100.1.2.3"]}
    }}"#;
    assert_eq!(
        crate::bootstrap::fleet::peer_address(status, "nova").as_deref(),
        Some("100.99.40.45"),
        "MagicDNS's name wins over the machine's own hostname"
    );
    assert_eq!(crate::bootstrap::fleet::peer_address(status, "lab"), None);
    assert_eq!(
        crate::bootstrap::fleet::peer_address(b"not json", "nova"),
        None
    );
}
