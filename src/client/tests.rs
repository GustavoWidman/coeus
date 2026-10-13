use std::error::Error;

use tokio::time::{Duration, timeout};

use crate::common::{
    proto::{DeployAccepted, DeployRequest, Packet},
    stream::{EncryptedListener, EncryptedStream},
};

use super::CoeusClient;

type TestResult<T = ()> = Result<T, Box<dyn Error + Send + Sync>>;

const PSK: [u8; 32] = [0x42; 32];

#[tokio::test]
async fn listener_handles_response_while_server_keeps_connection_open() -> TestResult {
    timeout(Duration::from_secs(2), async {
        let listener = EncryptedListener::bind("127.0.0.1:0".parse()?, &PSK).await?;
        let address = listener.local_addr()?;
        let (release_server, server_release) = tokio::sync::oneshot::channel();

        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await?;
            match stream.recv().await? {
                Some(Packet::DeployRequest(request)) => {
                    assert_eq!(request.rev, "abc123");
                }
                other => panic!("unexpected request: {other:?}"),
            }
            stream
                .send(Packet::DeployAccepted(Box::new(DeployAccepted {
                    job_id: 42,
                    message: "accepted".into(),
                })))
                .await?;
            let _ = server_release.await;
            Ok::<_, Box<dyn Error + Send + Sync>>(())
        });

        let stream = EncryptedStream::connect(address, &PSK).await?;
        let mut client = CoeusClient {
            stream,
            usable: true,
        };
        client
            .send(DeployRequest {
                request_id: rand::random(),
                rev: "abc123".into(),
                dry_run: true,
                clean_substituters: false,
            })
            .await?;
        client.listen().await?;
        release_server
            .send(())
            .expect("server should still be waiting after its response");
        server.await??;
        Ok::<_, Box<dyn Error + Send + Sync>>(())
    })
    .await??;
    Ok(())
}

#[tokio::test]
async fn listener_errors_if_server_closes_before_a_response() -> TestResult {
    timeout(Duration::from_secs(2), async {
        let listener = EncryptedListener::bind("127.0.0.1:0".parse()?, &PSK).await?;
        let address = listener.local_addr()?;

        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await?;
            match stream.recv().await? {
                Some(Packet::DeployRequest(request)) => {
                    assert_eq!(request.rev, "abc123");
                }
                other => panic!("unexpected request: {other:?}"),
            }
            drop(stream);
            Ok::<_, Box<dyn Error + Send + Sync>>(())
        });

        let stream = EncryptedStream::connect(address, &PSK).await?;
        let mut client = CoeusClient {
            stream,
            usable: true,
        };
        client
            .send(DeployRequest {
                request_id: rand::random(),
                rev: "abc123".into(),
                dry_run: true,
                clean_substituters: false,
            })
            .await?;
        let error = client
            .listen()
            .await
            .expect_err("closing before a response must fail");
        assert!(
            error
                .to_string()
                .contains("closed before sending a response")
        );
        server.await??;
        Ok::<_, Box<dyn Error + Send + Sync>>(())
    })
    .await??;
    Ok(())
}

#[tokio::test]
async fn sends_and_listens_concurrently_through_the_client_api() -> TestResult {
    timeout(Duration::from_secs(2), async {
        let listener = EncryptedListener::bind("127.0.0.1:0".parse()?, &PSK).await?;
        let address = listener.local_addr()?;

        let (release_server, server_release) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await?;
            match stream.recv().await? {
                Some(Packet::DeployRequest(request)) => {
                    assert_eq!(request.rev, "abc123");
                }
                other => panic!("unexpected request: {other:?}"),
            }
            stream
                .send(Packet::DeployAccepted(Box::new(DeployAccepted {
                    job_id: 42,
                    message: "accepted".into(),
                })))
                .await?;
            let _ = server_release.await;
            Ok::<_, Box<dyn Error + Send + Sync>>(())
        });

        let stream = EncryptedStream::connect(address, &PSK).await?;
        let mut client = CoeusClient {
            stream,
            usable: true,
        };
        client
            .send_and_listen(DeployRequest {
                request_id: rand::random(),
                rev: "abc123".into(),
                dry_run: true,
                clean_substituters: false,
            })
            .await?;
        release_server
            .send(())
            .expect("server should still be waiting after its response");
        server.await??;
        Ok::<_, Box<dyn Error + Send + Sync>>(())
    })
    .await??;
    Ok(())
}

#[tokio::test]
async fn cancelled_request_poisoning_prevents_consuming_a_stale_ack() -> TestResult {
    timeout(Duration::from_secs(2), async {
        let listener = EncryptedListener::bind("127.0.0.1:0".parse()?, &PSK).await?;
        let address = listener.local_addr()?;
        let (seen, received) = tokio::sync::oneshot::channel();
        let (release, released) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await?;
            stream.recv().await?.expect("request should arrive");
            let _ = seen.send(());
            let _ = released.await;
            Ok::<_, Box<dyn Error + Send + Sync>>(())
        });
        let stream = EncryptedStream::connect(address, &PSK).await?;
        let mut client = CoeusClient {
            stream,
            usable: true,
        };
        let mut pending = Box::pin(client.request(DeployRequest {
            request_id: rand::random(),
            rev: "abc123".into(),
            dry_run: false,
            clean_substituters: false,
        }));
        tokio::select! {
            _ = received => {},
            result = &mut pending => panic!("request should await acknowledgement: {result:?}"),
        }
        drop(pending);
        assert!(!client.usable);
        assert!(
            client
                .request(DeployRequest {
                    request_id: rand::random(),
                    rev: "retry".into(),
                    dry_run: false,
                    clean_substituters: false
                })
                .await
                .is_err()
        );
        assert!(
            client
                .send(DeployRequest {
                    request_id: rand::random(),
                    rev: "retry".into(),
                    dry_run: false,
                    clean_substituters: false
                })
                .await
                .is_err()
        );
        let _ = release.send(());
        server.await??;
        Ok::<_, Box<dyn Error + Send + Sync>>(())
    })
    .await??;
    Ok(())
}

#[tokio::test]
async fn initial_handshake_deadline_covers_a_silent_tcp_peer() -> TestResult {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let peer = tokio::spawn(async move {
        let (_stream, _) = listener.accept().await.unwrap();
        std::future::pending::<()>().await;
    });
    let result = timeout(
        Duration::from_secs(1),
        CoeusClient::connect(address, &PSK, Duration::from_millis(40)),
    )
    .await?;
    assert!(
        result
            .err()
            .expect("silent peer must time out")
            .to_string()
            .contains("handshake deadline")
    );
    peer.abort();
    let _ = peer.await;
    Ok(())
}
