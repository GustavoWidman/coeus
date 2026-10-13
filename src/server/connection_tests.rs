//! All fixtures are ephemeral loopback sockets and a never-called executor.
use super::CoeusServer;
use crate::{
    common::{proto::*, stream::EncryptedStream},
    jobs::{DeploymentExecutor, JobContext},
};
use eyre::Result;
use futures_util::future::BoxFuture;
use std::{sync::Arc, time::Duration};
use tokio::{
    io::AsyncReadExt,
    net::TcpStream,
    task::JoinHandle,
    time::{sleep, timeout},
};
use tokio_util::sync::CancellationToken;

const TEST_PSK: [u8; 32] = [0x57; 32];

struct NeverDeploy;
impl DeploymentExecutor for NeverDeploy {
    fn execute(&self, _: DeployRequest, _: JobContext) -> BoxFuture<'_, Result<()>> {
        Box::pin(async { panic!("connection fixtures never request deployment") })
    }
}

struct Fixture {
    address: std::net::SocketAddr,
    shutdown: CancellationToken,
    task: JoinHandle<Result<()>>,
}
impl Fixture {
    async fn start() -> Result<Self> {
        let server =
            CoeusServer::with_executor("127.0.0.1:0".parse()?, &TEST_PSK, Arc::new(NeverDeploy))
                .await?;
        let address = server.local_addr()?;
        let shutdown = CancellationToken::new();
        let task = tokio::spawn(server.run_until_cancelled(shutdown.clone()));
        Ok(Self {
            address,
            shutdown,
            task,
        })
    }

    async fn stop(mut self) -> Result<()> {
        self.shutdown.cancel();
        timeout(Duration::from_secs(2), &mut self.task).await???;
        Ok(())
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.shutdown.cancel();
        // Tests still reap successful shutdown through stop(). Abort on assertion failure.
        self.task.abort();
    }
}

async fn assert_pending(socket: &mut TcpStream) -> Result<()> {
    let mut byte = [0];
    assert!(
        timeout(Duration::from_millis(100), socket.read(&mut byte))
            .await
            .is_err(),
        "a pending Noise handshake closed before its deadline"
    );
    Ok(())
}

async fn assert_status(stream: &mut EncryptedStream) -> Result<()> {
    stream
        .send(StatusRequest {
            job_id: None,
            after: None,
        })
        .await?;
    assert!(matches!(
        stream.recv().await?,
        Some(Packet::StatusResponse(_))
    ));
    Ok(())
}

#[tokio::test]
async fn delayed_valid_handshake_survives_unrelated_authenticated_disconnect() -> Result<()> {
    timeout(Duration::from_secs(8), async {
        let fixture = Fixture::start().await?;
        for _ in 0..5 {
            let mut existing = EncryptedStream::connect(fixture.address, &TEST_PSK).await?;
            assert_status(&mut existing).await?;
            let mut pending = TcpStream::connect(fixture.address).await?;
            // Exercise a real delay before sending a valid Noise message.
            assert_pending(&mut pending).await?;
            drop(existing);
            assert_pending(&mut pending).await?;
            let mut connected = EncryptedStream::initiate(pending, &TEST_PSK).await?;
            assert_status(&mut connected).await?;
        }
        fixture.stop().await
    })
    .await??;
    Ok(())
}

#[tokio::test]
async fn slow_handshake_does_not_block_authenticated_status_or_shutdown() -> Result<()> {
    timeout(Duration::from_secs(5), async {
        let fixture = Fixture::start().await?;
        let mut pending = TcpStream::connect(fixture.address).await?;
        assert_pending(&mut pending).await?;
        let mut healthy = EncryptedStream::connect(fixture.address, &TEST_PSK).await?;
        assert_status(&mut healthy).await?;
        fixture.stop().await?;
        let mut byte = [0];
        assert_eq!(
            pending.read(&mut byte).await?,
            0,
            "shutdown must drop pending socket"
        );
        assert!(
            healthy.recv().await?.is_none(),
            "shutdown must drop authenticated socket"
        );
        Ok::<_, eyre::Report>(())
    })
    .await??;
    Ok(())
}

#[tokio::test]
async fn pending_and_authenticated_sessions_share_connection_cap() -> Result<()> {
    timeout(Duration::from_secs(5), async {
        let fixture = Fixture::start().await?;
        let mut healthy = EncryptedStream::connect(fixture.address, &TEST_PSK).await?;
        assert_status(&mut healthy).await?;
        let mut pending = Vec::new();
        for _ in 1..super::connection::MAX_CONNECTIONS {
            pending.push(TcpStream::connect(fixture.address).await?);
        }
        let address = fixture.address;
        let mut overflow = tokio::spawn(EncryptedStream::connect(address, &TEST_PSK));
        sleep(Duration::from_millis(100)).await;
        assert!(
            !overflow.is_finished(),
            "connection cap excludes pending handshakes"
        );
        // Established sessions still work at the combined cap.
        assert_status(&mut healthy).await?;
        drop(healthy);
        let mut admitted = timeout(Duration::from_secs(2), &mut overflow).await???;
        assert_status(&mut admitted).await?;
        fixture.stop().await?;
        Ok::<_, eyre::Report>(())
    })
    .await??;
    Ok(())
}

#[tokio::test]
async fn silent_handshake_expires_without_closing_healthy_session() -> Result<()> {
    timeout(Duration::from_secs(14), async {
        let fixture = Fixture::start().await?;
        let mut pending = TcpStream::connect(fixture.address).await?;
        let mut healthy = EncryptedStream::connect(fixture.address, &TEST_PSK).await?;
        assert_status(&mut healthy).await?;
        assert_pending(&mut pending).await?;
        let mut byte = [0];
        assert_eq!(
            timeout(
                super::connection::HANDSHAKE_DEADLINE + Duration::from_secs(1),
                pending.read(&mut byte)
            )
            .await??,
            0
        );
        assert_status(&mut healthy).await?;
        fixture.stop().await
    })
    .await??;
    Ok(())
}
