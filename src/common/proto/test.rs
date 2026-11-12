use crate::common::{
    model::{JobId, JobSelector},
    proto::envelope::PacketEnvelope,
};

use super::*;

#[test]
fn packet_round_trips_with_stable_kind() {
    let packet = Packet::from(ResolveRequest {
        git_ref: "main".into(),
    });

    assert_eq!(packet.kind(), 0x0105);

    let encoded = postcard::to_stdvec(&packet).unwrap();
    assert_eq!(
        encoded,
        vec![0x85, 0x02, 0x05, 0x04, b'm', b'a', b'i', b'n']
    );

    let Packet::ResolveRequest(request) = postcard::from_bytes(&encoded).unwrap() else {
        panic!("decoded the wrong packet variant");
    };
    assert_eq!(request.git_ref, "main");
}

#[test]
fn packet_envelope_round_trips() {
    let envelope = PacketEnvelope::new(Packet::from(WatchRequest { follow: true }));

    let encoded = postcard::to_stdvec(&envelope).unwrap();
    let decoded: PacketEnvelope = postcard::from_bytes(&encoded).unwrap();
    decoded.validate().unwrap();

    assert_eq!(decoded.packet.kind(), 0x0200);
}

#[test]
fn rejects_unknown_packet_kinds() {
    let encoded = [0xff, 0x7f, 0x00];
    assert!(postcard::from_bytes::<Packet>(&encoded).is_err());
}

#[test]
fn daemons_that_cannot_draw_still_read_an_attach_request() {
    let envelope = PacketEnvelope::new(Packet::from(AttachRequest {
        job: JobSelector::Id(JobId(6)),
        from: 42,
        terminal: Some(TerminalSize {
            columns: 140,
            rows: 40,
        }),
    }));
    let encoded = postcard::to_stdvec(&envelope).unwrap();

    let (version, kind, payload): (u8, u16, Vec<u8>) = postcard::from_bytes(&encoded).unwrap();
    assert_eq!((version, kind), (PROTOCOL_VERSION, 0x0202));

    let request: (JobSelector, u64) = postcard::from_bytes(&payload).unwrap();
    assert_eq!(request, (JobSelector::Id(JobId(6)), 42));
}
