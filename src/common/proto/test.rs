use crate::common::proto::envelope::PacketEnvelope;

use super::*;

#[test]
fn packet_round_trips_with_stable_kind() {
    let packet = Packet::DeployRequest(Box::new(DeployRequest {
        revision: "abc123".into(),
        dry_run: true,
    }));

    assert_eq!(packet.kind(), 0x0100);

    let encoded = postcard::to_stdvec(&packet).unwrap();
    assert_eq!(
        encoded,
        vec![
            0x80, 0x02, 0x08, 0x06, b'a', b'b', b'c', b'1', b'2', b'3', 0x01
        ]
    );
    let decoded: Packet = postcard::from_bytes(&encoded).unwrap();

    match decoded {
        Packet::DeployRequest(request) => {
            assert_eq!(request.revision, "abc123");
            assert!(request.dry_run);
        }
        Packet::DeployAccepted(_) => panic!("decoded the wrong packet variant"),
    }
}

#[test]
fn packet_envelope_round_trips() {
    let envelope = PacketEnvelope::new(Packet::DeployAccepted(Box::new(DeployAccepted {
        message: "accepted".into(),
    })));

    let encoded = postcard::to_stdvec(&envelope).unwrap();
    let decoded: PacketEnvelope = postcard::from_bytes(&encoded).unwrap();
    decoded.validate().unwrap();

    assert_eq!(decoded.packet.kind(), 0x0101);
}
