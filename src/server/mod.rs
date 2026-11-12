mod fit;
mod session;
mod shutdown;

use std::{net::SocketAddr, sync::Arc, time::Duration};

use colored::Colorize;
use eyre::{Context, Result};
use log::{debug, error, info};
use tokio::{task::JoinSet, time::timeout};
use tokio_util::sync::CancellationToken;

use crate::{
    common::stream::{EncryptedListener, PendingConnection},
    config::Key,
    engine::{EngineHandle, Host},
};

pub const HANDSHAKE_DEADLINE: Duration = Duration::from_secs(10);

pub struct CoeusServer<H: Host> {
    listener: EncryptedListener,
    engine: EngineHandle<H>,
}

impl<H: Host> CoeusServer<H> {
    pub async fn bind(address: SocketAddr, key: &Key, engine: EngineHandle<H>) -> Result<Self> {
        let listener =
            EncryptedListener::bind(address, key.as_bytes(), engine.host().clone()).await?;
        info!(
            "coeus server listening on {}",
            listener.local_addr()?.to_string().magenta()
        );

        Ok(Self { listener, engine })
    }

    pub fn local_addr(&self) -> std::io::Result<SocketAddr> {
        self.listener.local_addr()
    }

    /// serves until a termination signal arrives or `restart` is cancelled
    /// (the engine wants to exit into a newer coeusd), then drains
    pub async fn run(self, restart: CancellationToken) -> Result<()> {
        let mut signals = shutdown::Signals::register()?;
        let shutdown = CancellationToken::new();

        let server = self.serve(shutdown.clone());
        tokio::pin!(server);

        tokio::select! {
            result = &mut server => result,
            signal = signals.wait() => {
                shutdown.cancel();
                let served = server.await;
                signal.and(served)
            },
            _ = restart.cancelled() => {
                info!("exiting so the service manager starts the new coeusd");
                shutdown.cancel();
                server.await
            },
        }
    }

    async fn serve(self, shutdown: CancellationToken) -> Result<()> {
        let server = Arc::new(self);
        let mut connections = JoinSet::new();

        loop {
            tokio::select! {
                biased;
                _ = shutdown.cancelled() => break,
                Some(_) = connections.join_next(), if !connections.is_empty() => {},
                // Tokio TCP accept is cancellation-safe. Noise belongs to the spawned
                // task, not a sibling select branch; reaping another task cannot close it.
                accepted = server.listener.listen() => match accepted {
                    Ok(pending) => {
                        let server = server.clone();
                        connections.spawn(async move {
                            if let Err(error) = server.handle_connection(pending).await {
                                debug!("connection ended: {error}");
                            }
                        });
                    }
                    Err(error) => error!("TCP accept failed: {error}"),
                },
            }
        }

        connections.abort_all();
        while connections.join_next().await.is_some() {}
        Ok(())
    }

    async fn handle_connection(&self, pending: PendingConnection) -> Result<()> {
        let stream = timeout(HANDSHAKE_DEADLINE, pending.authenticate())
            .await
            .wrap_err("Noise handshake deadline reached")??;

        debug!("{} connected", stream.peer());
        session::Session::new(stream, self.engine.clone())
            .run()
            .await
    }
}

#[cfg(test)]
mod test;
