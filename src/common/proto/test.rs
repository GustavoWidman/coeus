use crate::common::proto::envelope::PacketEnvelope;

use super::*;

#[test]
fn packet_round_trips_with_stable_kind() {
    let packet = Packet::DeployRequest(Box::new(DeployRequest {
        request_id: 42,
        rev: "abc123".into(),
        dry_run: true,
        clean_substituters: false,
    }));

    assert_eq!(packet.kind(), 0x0100);

    let encoded = postcard::to_stdvec(&packet).unwrap();
    assert_eq!(
        encoded,
        vec![
            0x80, 0x02, 0x0a, 42, 0x06, b'a', b'b', b'c', b'1', b'2', b'3', 0x01, 0x00
        ]
    );
    let decoded: Packet = postcard::from_bytes(&encoded).unwrap();

    match decoded {
        Packet::DeployRequest(request) => {
            assert_eq!(request.rev, "abc123");
            assert!(request.dry_run);
        }
        _ => panic!("decoded the wrong packet variant"),
    }
}

#[test]
fn packet_envelope_round_trips() {
    let envelope = PacketEnvelope::new(Packet::DeployAccepted(Box::new(DeployAccepted {
        job_id: 42,
        message: "accepted".into(),
    })));

    let encoded = postcard::to_stdvec(&envelope).unwrap();
    let decoded: PacketEnvelope = postcard::from_bytes(&encoded).unwrap();
    decoded.validate().unwrap();

    assert_eq!(decoded.packet.kind(), 0x0101);
}

#[test]
fn v3_ack_and_status_request_have_fixed_encoding() {
    let envelope = PacketEnvelope::new(
        DeployAccepted {
            job_id: 42,
            message: "ok".into(),
        }
        .into(),
    );
    assert_eq!(
        postcard::to_stdvec(&envelope).unwrap(),
        vec![3, 0x81, 2, 4, 42, 2, b'o', b'k']
    );
    let packet: Packet = StatusRequest {
        job_id: Some(42),
        after: None,
    }
    .into();
    assert_eq!(
        postcard::to_stdvec(&packet).unwrap(),
        vec![0x80, 4, 3, 1, 42, 0]
    );
    let invalid = PacketEnvelope {
        protocol_version: 1,
        packet,
    };
    assert!(invalid.validate().is_err());
}

#[test]
fn query_and_cancellation_packets_round_trip() {
    let packets: Vec<Packet> = vec![
        StatusRequest {
            job_id: None,
            after: Some(42),
        }
        .into(),
        StatusResponse {
            jobs: vec![],
            next_after: None,
        }
        .into(),
        LogsRequest {
            job_id: 42,
            after: 12,
        }
        .into(),
        LogsResponse {
            job_id: 42,
            entries: vec![LogEntry {
                cursor: 13,
                source: LogSource::Stderr,
                text: "error".into(),
            }],
            next_cursor: 13,
            gap: false,
            finished: true,
        }
        .into(),
        CancelRequest { job_id: 42 }.into(),
        RequestRejected {
            message: "queue full".into(),
        }
        .into(),
    ];
    for packet in packets {
        let encoded = postcard::to_stdvec(&packet).unwrap();
        let decoded: Packet = postcard::from_bytes(&encoded).unwrap();
        assert_eq!(decoded.kind(), packet.kind());
        assert_eq!(postcard::to_stdvec(&decoded).unwrap(), encoded);
    }
}

#[test]
fn bounded_response_pages_fit_a_noise_frame() {
    let jobs = (0..32)
        .map(|_| JobStatus {
            job_id: u64::MAX,
            revision: "r".repeat(256),
            commit: Some("f".repeat(40)),
            phase: DeployPhase::Failed,
            cancel_requested: true,
            message: Some("e".repeat(1024)),
        })
        .collect();
    let status = PacketEnvelope::new(
        StatusResponse {
            jobs,
            next_after: Some(u64::MAX),
        }
        .into(),
    );
    let entries = (0..32)
        .map(|_| LogEntry {
            cursor: u64::MAX,
            source: LogSource::Stderr,
            text: "x".repeat(1024),
        })
        .collect();
    let logs = PacketEnvelope::new(
        LogsResponse {
            job_id: u64::MAX,
            entries,
            next_cursor: u64::MAX,
            gap: true,
            finished: true,
        }
        .into(),
    );
    for response in [status, logs] {
        assert!(postcard::to_stdvec(&response).unwrap().len() <= u16::MAX as usize - 16);
    }
}
