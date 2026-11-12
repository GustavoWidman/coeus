use std::error::Error;

use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    time::{Duration, timeout},
};

use crate::common::{
    model::{HostInfo, Platform},
    proto::{AttachScreen, Error as ErrorPacket, Packet, ResolveRequest},
};

use super::{EncryptedListener, EncryptedStream};

type TestResult<T = ()> = Result<T, Box<dyn Error + Send + Sync>>;

const PSK: [u8; 32] = [0x42; 32];
const WRONG_PSK: [u8; 32] = [0x24; 32];

fn host() -> HostInfo {
    HostInfo {
        hostname: "test".parse().unwrap(),
        platform: Platform::NixOs,
        system: "x86_64-linux".into(),
        version: "0.0.0".into(),
    }
}

async fn bind() -> TestResult<EncryptedListener> {
    Ok(EncryptedListener::bind("127.0.0.1:0".parse()?, &PSK, host()).await?)
}

async fn echo(payload: Vec<u8>) -> TestResult<Vec<u8>> {
    let listener = bind().await?;
    let address = listener.local_addr()?;
    let expected = payload.clone();

    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await?;

        let mut received = vec![0u8; expected.len()];
        stream.read_exact(&mut received).await?;
        stream.write_all(&received).await?;
        stream.flush().await?;

        Ok::<_, Box<dyn Error + Send + Sync>>(received)
    });

    let (mut client, _) = EncryptedStream::connect(address, &PSK).await?;
    client.write_all(&payload).await?;
    client.flush().await?;

    let mut echoed = vec![0u8; payload.len()];
    client.read_exact(&mut echoed).await?;

    assert_eq!(server.await??, payload);
    Ok(echoed)
}

#[tokio::test]
async fn round_trips_plaintext_through_an_encrypted_stream() -> TestResult {
    let payload = b"deploy request".to_vec();
    assert_eq!(echo(payload.clone()).await?, payload);
    Ok(())
}

#[tokio::test]
async fn splits_large_writes_across_encrypted_frames() -> TestResult {
    let frame_payload = EncryptedStream::MAX_FRAME - EncryptedStream::TAG_LEN;
    let payload: Vec<u8> = (0..(frame_payload * 2 + 17))
        .map(|index| (index % 251) as u8)
        .collect();

    assert_eq!(echo(payload.clone()).await?, payload);
    Ok(())
}

#[tokio::test]
async fn serves_plaintext_in_small_read_buffers() -> TestResult {
    let payload = b"response payload spanning several reads".to_vec();
    let listener = bind().await?;
    let address = listener.local_addr()?;
    let expected = payload.clone();

    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await?;
        stream.write_all(&expected).await?;
        stream.flush().await?;
        Ok::<_, Box<dyn Error + Send + Sync>>(())
    });

    let (mut client, _) = EncryptedStream::connect(address, &PSK).await?;
    let mut actual = Vec::new();
    let mut buffer = [0u8; 3];

    while actual.len() < payload.len() {
        let count = client.read(&mut buffer).await?;
        assert!(
            count > 0,
            "stream ended before the complete payload arrived"
        );
        actual.extend_from_slice(&buffer[..count]);
    }

    assert_eq!(actual, payload);
    server.await??;
    Ok(())
}

#[tokio::test]
async fn rejects_a_mismatched_psk() -> TestResult {
    let listener = bind().await?;
    let address = listener.local_addr()?;

    let server = tokio::spawn(async move {
        let result = listener.accept().await;
        Ok::<_, Box<dyn Error + Send + Sync>>(result.is_ok())
    });

    let client = timeout(
        Duration::from_secs(2),
        EncryptedStream::connect(address, &WRONG_PSK),
    )
    .await?;

    let Err(error) = client else {
        panic!("a mismatched PSK must fail the handshake");
    };
    assert!(
        format!("{error:#}").contains("fleet key"),
        "the error names the likely cause: {error:#}"
    );
    let _server_completed = server.await??;
    Ok(())
}

#[tokio::test]
async fn empty_writes_are_noops() -> TestResult {
    let listener = bind().await?;
    let address = listener.local_addr()?;

    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await?;
        let mut received = [0u8; 1];
        stream.read_exact(&mut received).await?;
        Ok::<_, Box<dyn Error + Send + Sync>>(received)
    });

    let (mut client, _) = EncryptedStream::connect(address, &PSK).await?;
    assert_eq!(client.write(&[]).await?, 0);
    client.write_all(b"x").await?;
    client.flush().await?;

    assert_eq!(server.await??, [b'x']);
    Ok(())
}

#[tokio::test]
async fn round_trips_registered_packets() -> TestResult {
    let listener = bind().await?;
    let address = listener.local_addr()?;

    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await?;
        let packet = stream.recv().await?;

        match packet {
            Some(Packet::ResolveRequest(request)) => {
                assert_eq!(request.git_ref, "abc123");
            }
            other => panic!("unexpected packet: {other:?}"),
        }

        stream
            .send(ErrorPacket {
                message: "accepted".into(),
            })
            .await?;

        Ok::<_, Box<dyn Error + Send + Sync>>(())
    });

    let (mut client, _) = EncryptedStream::connect(address, &PSK).await?;
    client
        .send(ResolveRequest {
            git_ref: "abc123".into(),
        })
        .await?;

    match client.recv().await? {
        Some(Packet::Error(accepted)) => {
            assert_eq!(accepted.message, "accepted");
        }
        other => panic!("unexpected packet: {other:?}"),
    }

    server.await??;
    Ok(())
}

#[tokio::test]
async fn owned_halves_send_and_receive_packets_concurrently() -> TestResult {
    const PACKET_COUNT: usize = 8;

    timeout(Duration::from_secs(2), async {
        let listener = bind().await?;
        let address = listener.local_addr()?;

        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await?;
            let (reader, writer) = stream.split();

            let receive_requests = async {
                for sequence in 0..PACKET_COUNT {
                    match reader.recv().await? {
                        Some(Packet::ResolveRequest(request)) => {
                            assert_eq!(request.git_ref, format!("revision-{sequence}"));
                        }
                        other => panic!("unexpected request: {other:?}"),
                    }
                }
                Ok::<_, eyre::Report>(())
            };

            let send_responses = async {
                for sequence in 0..PACKET_COUNT {
                    writer
                        .send(ErrorPacket {
                            message: format!("accepted-{sequence}"),
                        })
                        .await?;
                }
                Ok::<_, eyre::Report>(())
            };

            tokio::try_join!(receive_requests, send_responses)?;
            Ok::<_, Box<dyn Error + Send + Sync>>(())
        });

        let (mut stream, hello) = EncryptedStream::connect(address, &PSK).await?;
        assert_eq!(hello.host, host());
        let (reader, writer) = stream.split();

        let send_requests = async {
            for sequence in 0..PACKET_COUNT {
                writer
                    .send(ResolveRequest {
                        git_ref: format!("revision-{sequence}"),
                    })
                    .await?;
            }
            Ok::<_, eyre::Report>(())
        };

        let receive_responses = async {
            for sequence in 0..PACKET_COUNT {
                match reader.recv().await? {
                    Some(Packet::Error(response)) => {
                        assert_eq!(response.message, format!("accepted-{sequence}"));
                    }
                    other => panic!("unexpected response: {other:?}"),
                }
            }
            Ok::<_, eyre::Report>(())
        };

        tokio::try_join!(send_requests, receive_responses)?;
        server.await??;
        Ok::<_, Box<dyn Error + Send + Sync>>(())
    })
    .await??;
    Ok(())
}

#[tokio::test]
async fn the_fullest_screen_fits_one_frame() -> TestResult {
    let listener = bind().await?;
    let address = listener.local_addr()?;
    let bytes = vec![0xff; AttachScreen::MAX];
    let sent = bytes.clone();

    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await?;
        stream.send(AttachScreen { bytes: sent }).await?;
        Ok::<_, Box<dyn Error + Send + Sync>>(())
    });

    let (mut client, _) = EncryptedStream::connect(address, &PSK).await?;
    match client.recv().await? {
        Some(Packet::AttachScreen(screen)) => assert_eq!(screen.bytes, bytes),
        other => panic!("unexpected packet: {other:?}"),
    }
    server.await??;
    Ok(())
}
