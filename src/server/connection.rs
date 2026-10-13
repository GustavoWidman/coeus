use super::CoeusServer;
use crate::common::stream::PendingConnection;
use eyre::{Result, WrapErr};
use std::time::Duration;

// Handshaking and established sessions share this cap and the same task owner.
pub(super) const MAX_CONNECTIONS: usize = 64;
pub(super) const HANDSHAKE_DEADLINE: Duration = Duration::from_secs(10);

impl CoeusServer {
    pub(super) async fn authenticate_and_handle(&self, pending: PendingConnection) -> Result<()> {
        let stream = tokio::time::timeout(HANDSHAKE_DEADLINE, pending.authenticate())
            .await
            .wrap_err("Noise handshake deadline reached")??;
        self.handle_connection(stream).await
    }
}
