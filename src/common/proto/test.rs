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
fn v5_ack_and_status_request_have_fixed_encoding() {
    let envelope = PacketEnvelope::new(
        DeployAccepted {
            job_id: 42,
            message: "ok".into(),
        }
        .into(),
    );
    assert_eq!(
        postcard::to_stdvec(&envelope).unwrap(),
        vec![6, 0x81, 2, 4, 42, 2, b'o', b'k']
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

#[test]
fn terminal_packets_have_deliberate_wire_encoding_and_fit_frame() {
    let request: Packet = TerminalRequest {
        job_id: 42,
        after: Some(7),
        size: TerminalSize { rows: 24, cols: 80 },
    }
    .into();
    // Packet macro wraps a postcard payload with kind + payload length.
    assert_eq!(
        postcard::to_stdvec(&request).unwrap(),
        vec![0x86, 4, 5, 42, 1, 7, 24, 80]
    );
    let response: Packet = TerminalResponse {
        job_id: 42,
        bytes: vec![0x1b, b'[', b'm', 0xff],
        next_cursor: 11,
        gap: false,
        available: true,
        finished: false,
        error: None,
    }
    .into();
    assert_eq!(
        postcard::to_stdvec(&response).unwrap(),
        vec![0x87, 4, 11, 42, 4, 0x1b, b'[', b'm', 0xff, 11, 0, 1, 0, 0]
    );
    let encoded = postcard::to_stdvec(&response).unwrap();
    let decoded: Packet = postcard::from_bytes(&encoded).unwrap();
    assert_eq!(postcard::to_stdvec(&decoded).unwrap(), encoded);
    let maximum = PacketEnvelope::new(
        TerminalResponse {
            job_id: u64::MAX,
            bytes: vec![0xff; TERMINAL_PAGE_BYTES],
            next_cursor: u64::MAX,
            gap: true,
            available: true,
            finished: true,
            error: Some("e".repeat(1024)),
        }
        .into(),
    );
    assert!(postcard::to_stdvec(&maximum).unwrap().len() < u16::MAX as usize - 16);
    let wrong_version = PacketEnvelope {
        protocol_version: 3,
        packet: request,
    };
    assert!(wrong_version.validate().is_err());
}

#[test]
fn v5_identity_request_and_appended_phase_preserve_old_disk_encoding() {
    assert_eq!(postcard::to_stdvec(&DeployPhase::Switch).unwrap(), vec![3]);
    assert_eq!(
        postcard::to_stdvec(&DeployPhase::Interrupted).unwrap(),
        vec![7]
    );
    assert_eq!(
        postcard::to_stdvec(&DeployPhase::RequiresManualSwitch).unwrap(),
        vec![8]
    );
    let packet: Packet = IdentityRequest {}.into();
    assert_eq!(packet.kind(), 0x0300);
    assert_eq!(postcard::to_stdvec(&packet).unwrap(), vec![0x80, 0x06, 0]);
    let encoded = postcard::to_stdvec(&PacketEnvelope::new(packet)).unwrap();
    assert_eq!(encoded[0], 6);
    let decoded: PacketEnvelope = postcard::from_bytes(&encoded).unwrap();
    decoded.validate().unwrap();
}

#[test]
fn v6_machine_queries_have_unique_kinds_and_exact_hello_encoding() {
    let hello: Packet = HelloRequest {}.into();
    assert_eq!(hello.kind(), 0x0302);
    assert_eq!(
        postcard::to_stdvec(&PacketEnvelope::new(hello)).unwrap(),
        vec![6, 0x82, 6, 0]
    );
    let packets: Vec<Packet> = vec![
        HelloResponse {
            protocol_version: PROTOCOL_VERSION,
            version: "fixture".into(),
        }
        .into(),
        MachineStatusRequest {}.into(),
        MachineStatusResponse {
            active: None,
            latest: None,
        }
        .into(),
        SharedTerminalRequest {
            after: Some(0),
            size: Default::default(),
        }
        .into(),
        SharedTerminalResponse {
            chunks: vec![SharedTerminalChunk {
                job_id: 1,
                bytes: b"\x1b[31m\r".to_vec(),
            }],
            next_cursor: 6,
            gap: false,
        }
        .into(),
    ];
    for packet in packets {
        let bytes = postcard::to_stdvec(&packet).unwrap();
        let decoded: Packet = postcard::from_bytes(&bytes).unwrap();
        assert_eq!(decoded.kind(), packet.kind());
        assert_eq!(postcard::to_stdvec(&decoded).unwrap(), bytes);
    }
}

#[test]
fn shared_terminal_maximum_chunk_overhead_fits_noise_frame() {
    let response = PacketEnvelope::new(
        SharedTerminalResponse {
            chunks: (0..TERMINAL_PAGE_BYTES)
                .map(|_| SharedTerminalChunk {
                    job_id: u64::MAX,
                    bytes: vec![0xff],
                })
                .collect(),
            next_cursor: u64::MAX,
            gap: false,
        }
        .into(),
    );
    assert!(postcard::to_stdvec(&response).unwrap().len() <= u16::MAX as usize - 16);
}
