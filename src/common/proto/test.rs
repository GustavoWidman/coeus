use crate::common::{
    model::{Action, JobId, JobSelector, NixOptions, Order, OrderId, Origin, Source},
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

#[test]
fn a_read_only_key_may_watch_and_read_but_never_make_the_daemon_work() {
    let terminal = TerminalSize {
        columns: 80,
        rows: 24,
    };
    let order = Order {
        id: OrderId(1),
        source: Source::Generation(3),
        action: Action::Switch,
        options: NixOptions::default(),
        origin: Origin::Follow,
        now: false,
    };
    let table: Vec<(Packet, bool)> = vec![
        (WatchRequest { follow: false }.into(), true),
        (WatchRequest { follow: true }.into(), true),
        (HistoryRequest { limit: 10 }.into(), true),
        (GenerationsRequest {}.into(), true),
        (
            AttachRequest {
                job: JobSelector::Current,
                from: 0,
                terminal: None,
            }
            .into(),
            true,
        ),
        (
            AttachRequest {
                job: JobSelector::Current,
                from: 0,
                terminal: Some(terminal),
            }
            .into(),
            false,
        ),
        (AttachResize { size: terminal }.into(), false),
        (DeployRequest { order }.into(), false),
        (
            CancelRequest {
                job: JobSelector::Current,
            }
            .into(),
            false,
        ),
        (
            ResolveRequest {
                git_ref: "main".into(),
            }
            .into(),
            false,
        ),
        (CatchUpRequest {}.into(), false),
        (RebootRequest {}.into(), false),
        (
            UploadBegin {
                nar_hash: format!("sha256-{}", "A".repeat(44)).parse().unwrap(),
                size: 0,
            }
            .into(),
            false,
        ),
        (UploadChunk { data: vec![0] }.into(), false),
        (HelloRequest {}.into(), false),
        (HelloFinished { compatible: true }.into(), false),
        (
            Error {
                message: "no".into(),
            }
            .into(),
            false,
        ),
        (Ack {}.into(), false),
    ];

    for (packet, allowed) in &table {
        assert!(Role::Full.allows(packet), "{:#06x}", packet.kind());
        assert_eq!(
            Role::Read.allows(packet),
            *allowed,
            "a read-only key sending {:#06x}",
            packet.kind()
        );
    }
}
