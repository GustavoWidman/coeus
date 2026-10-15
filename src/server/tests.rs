use super::CoeusServer;
use crate::{
    common::{proto::*, stream::EncryptedStream},
    jobs::{DeploymentExecutor, JobContext},
};
use eyre::Result;
use futures_util::future::BoxFuture;
use std::{sync::Arc, time::Duration};
use tokio::{
    sync::{Semaphore, mpsc},
    time::timeout,
};
use tokio_util::sync::CancellationToken;
const TEST_PSK: [u8; 32] = [0x42; 32];

struct FakeExecutor {
    started: mpsc::UnboundedSender<()>,
    release: Arc<Semaphore>,
}
impl DeploymentExecutor for FakeExecutor {
    fn execute(&self, _: DeployRequest, context: JobContext) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move {
            context.phase(DeployPhase::Build).await?;
            self.started.send(())?;
            tokio::select! {
                _ = context.cancelled() => context.check_cancelled(),
                permit = self.release.acquire() => { permit?.forget(); context.log(LogSource::Stdout, "fake build output").await; Ok(()) },
            }
        })
    }
}
fn request() -> DeployRequest {
    DeployRequest {
        request_id: rand::random(),
        rev: "test-revision".into(),
        dry_run: false,
        clean_substituters: false,
    }
}

#[tokio::test]
async fn encrypted_ack_disconnect_reconnect_status_logs_and_cancellation() -> Result<()> {
    timeout(Duration::from_secs(5), async {
        let (started, mut receiver) = mpsc::unbounded_channel();
        let release = Arc::new(Semaphore::new(0));
        let server = CoeusServer::with_executor(
            "127.0.0.1:0".parse()?,
            &TEST_PSK,
            Arc::new(FakeExecutor {
                started,
                release: release.clone(),
            }),
        )
        .await?;
        let address = server.local_addr()?;
        let shutdown = CancellationToken::new();
        let task = tokio::spawn(server.run_until_cancelled(shutdown.clone()));
        let mut client = EncryptedStream::connect(address, &TEST_PSK).await?;
        client.send(request()).await?;
        let Some(Packet::DeployAccepted(accepted)) = client.recv().await? else {
            panic!("missing acknowledgement");
        };
        let id = accepted.job_id;
        receiver.recv().await.unwrap();
        drop(client);
        let mut monitor = EncryptedStream::connect(address, &TEST_PSK).await?;
        monitor
            .send(StatusRequest {
                job_id: Some(id),
                after: None,
            })
            .await?;
        let Some(Packet::StatusResponse(status)) = monitor.recv().await? else {
            panic!("missing status");
        };
        assert_eq!(status.jobs[0].phase, DeployPhase::Build);
        monitor
            .send(LogsRequest {
                job_id: id,
                after: 0,
            })
            .await?;
        let Some(Packet::LogsResponse(logs)) = monitor.recv().await? else {
            panic!("missing logs");
        };
        assert_eq!(logs.entries[0].text, "Build");
        monitor.send(CancelRequest { job_id: id }).await?;
        let Some(Packet::StatusResponse(status)) = monitor.recv().await? else {
            panic!("missing cancel ack");
        };
        assert!(status.jobs[0].cancel_requested);
        // Unknown jobs are rejected without killing an authenticated connection.
        monitor
            .send(StatusRequest {
                job_id: Some(id.wrapping_add(1)),
                after: None,
            })
            .await?;
        assert!(matches!(
            monitor.recv().await?,
            Some(Packet::RequestRejected(_))
        ));
        monitor
            .send(StatusRequest {
                job_id: None,
                after: None,
            })
            .await?;
        assert!(matches!(
            monitor.recv().await?,
            Some(Packet::StatusResponse(_))
        ));
        shutdown.cancel();
        task.await??;
        Ok::<_, eyre::Report>(())
    })
    .await??;
    Ok(())
}

#[tokio::test]
async fn acceptance_survives_lost_ack_without_duplicate_replay() -> Result<()> {
    timeout(Duration::from_secs(5), async {
        let (started, mut receiver) = mpsc::unbounded_channel();
        let server = CoeusServer::with_executor(
            "127.0.0.1:0".parse()?,
            &TEST_PSK,
            Arc::new(FakeExecutor {
                started,
                release: Arc::new(Semaphore::new(0)),
            }),
        )
        .await?;
        let address = server.local_addr()?;
        let shutdown = CancellationToken::new();
        let task = tokio::spawn(server.run_until_cancelled(shutdown.clone()));
        let mut client = EncryptedStream::connect(address, &TEST_PSK).await?;
        client.send(request()).await?;
        drop(client); // ACK may be lost; NEVER resubmit automatically.
        receiver.recv().await.unwrap();
        let mut monitor = EncryptedStream::connect(address, &TEST_PSK).await?;
        monitor
            .send(StatusRequest {
                job_id: None,
                after: None,
            })
            .await?;
        let Some(Packet::StatusResponse(status)) = monitor.recv().await? else {
            panic!("missing status");
        };
        assert_eq!(status.jobs.len(), 1);
        shutdown.cancel();
        task.await??;
        Ok::<_, eyre::Report>(())
    })
    .await??;
    Ok(())
}

#[tokio::test]
async fn durable_lost_ack_retry_is_one_execution_and_recovered_handle() -> Result<()> {
    timeout(Duration::from_secs(10), async {
        let directory = tempfile::tempdir()?;
        let state = directory.path().join("state");
        let (started, mut receiver) = mpsc::unbounded_channel();
        let executor = Arc::new(FakeExecutor {
            started,
            release: Arc::new(Semaphore::new(0)),
        });
        let server = CoeusServer::with_durable_executor(
            "127.0.0.1:0".parse()?,
            &TEST_PSK,
            executor.clone(),
            state.clone(),
            "fixture".into(),
        )
        .await?;
        let address = server.local_addr()?;
        let shutdown = CancellationToken::new();
        let task = tokio::spawn(server.run_until_cancelled(shutdown.clone()));
        let original = request();
        let id = 1; // first server-assigned ID, independent of retry nonce
        let mut lost = EncryptedStream::connect(address, &TEST_PSK).await?;
        lost.send(original.clone()).await?;
        drop(lost);
        receiver.recv().await.unwrap();
        let mut retry = EncryptedStream::connect(address, &TEST_PSK).await?;
        retry.send(original.clone()).await?;
        let Some(Packet::DeployAccepted(accepted)) = retry.recv().await? else {
            panic!("retry must return original handle");
        };
        assert_eq!(accepted.job_id, id);
        let mut conflict = original.clone();
        conflict.rev = "different".into();
        retry.send(conflict.clone()).await?;
        assert!(matches!(
            retry.recv().await?,
            Some(Packet::RequestRejected(_))
        ));
        assert!(receiver.try_recv().is_err());
        shutdown.cancel();
        task.await??;
        drop(retry);
        let recovered = CoeusServer::with_durable_executor(
            "127.0.0.1:0".parse()?,
            &TEST_PSK,
            executor,
            state,
            "fixture".into(),
        )
        .await?;
        let address = recovered.local_addr()?;
        let shutdown = CancellationToken::new();
        let task = tokio::spawn(recovered.run_until_cancelled(shutdown.clone()));
        let mut retry = EncryptedStream::connect(address, &TEST_PSK).await?;
        retry.send(original).await?;
        let Some(Packet::DeployAccepted(accepted)) = retry.recv().await? else {
            panic!("durable retry must return handle");
        };
        assert_eq!(accepted.job_id, id);
        retry.send(conflict).await?;
        assert!(matches!(
            retry.recv().await?,
            Some(Packet::RequestRejected(_))
        ));
        retry
            .send(StatusRequest {
                job_id: Some(id),
                after: None,
            })
            .await?;
        let Some(Packet::StatusResponse(status)) = retry.recv().await? else {
            panic!("missing retained status");
        };
        assert_eq!(status.jobs[0].phase, DeployPhase::Cancelled);
        assert!(receiver.try_recv().is_err());
        shutdown.cancel();
        task.await??;
        Ok::<_, eyre::Report>(())
    })
    .await??;
    Ok(())
}

#[tokio::test]
async fn authenticated_machine_hello_status_and_shared_feed_require_no_job_id() -> Result<()> {
    timeout(Duration::from_secs(5), async {
        let server = CoeusServer::with_executor(
            "127.0.0.1:0".parse()?,
            &TEST_PSK,
            Arc::new(FakeExecutor {
                started: mpsc::unbounded_channel().0,
                release: Arc::new(Semaphore::new(0)),
            }),
        )
        .await?;
        let address = server.local_addr()?;
        let shutdown = CancellationToken::new();
        let task = tokio::spawn(server.run_until_cancelled(shutdown.clone()));
        let mut client = EncryptedStream::connect(address, &TEST_PSK).await?;
        client.send(HelloRequest {}).await?;
        let Some(Packet::HelloResponse(hello)) = client.recv().await? else {
            panic!("missing authenticated hello")
        };
        assert_eq!(hello.protocol_version, PROTOCOL_VERSION);
        assert_eq!(hello.version, env!("CARGO_PKG_VERSION"));
        client.send(MachineStatusRequest {}).await?;
        let Some(Packet::MachineStatusResponse(status)) = client.recv().await? else {
            panic!("missing machine status")
        };
        assert!(status.active.is_none() && status.latest.is_none());
        client
            .send(SharedTerminalRequest {
                after: None,
                size: Default::default(),
            })
            .await?;
        let Some(Packet::SharedTerminalResponse(page)) = client.recv().await? else {
            panic!("missing shared feed")
        };
        assert_eq!(page.next_cursor, 0);
        assert!(page.chunks.is_empty() && !page.gap);
        shutdown.cancel();
        task.await??;
        Ok::<_, eyre::Report>(())
    })
    .await??;
    Ok(())
}
