use eyre::{Result, bail, eyre};
use log::debug;

use crate::common::{
    model::HostInfo,
    proto::{HelloFinished, HelloRequest, HelloResponse, PROTOCOL_VERSION, Packet},
};

use super::EncryptedStream;

/// the three-way hello both ends run right after the NOISE handshake,
/// so that no packet ever reaches an incompatible peer.
impl EncryptedStream {
    /// client side: announce ourselves, read the server's versions and
    /// state whether we can talk to it.
    pub(super) async fn greet(&mut self) -> Result<HelloResponse> {
        self.send(HelloRequest::default()).await?;

        let Some(Packet::HelloResponse(response)) = self.recv().await? else {
            bail!("{} sent an unexpected packet during the hello", self.peer());
        };

        debug!(
            "{} is {} running coeus v{} (protocol v{})",
            self.peer(),
            response.host.hostname,
            response.host.version,
            response.protocol_version,
        );

        let compatible = response.protocol_version == PROTOCOL_VERSION;
        self.send(HelloFinished { compatible }).await?;

        if !compatible {
            bail!(
                "{} speaks protocol v{}, this client speaks v{PROTOCOL_VERSION}",
                self.peer(),
                response.protocol_version,
            );
        }

        Ok(*response)
    }

    /// server side: introduce `host`, then wait for the client's verdict.
    pub(super) async fn welcome(&mut self, host: &HostInfo) -> Result<()> {
        let Some(Packet::HelloRequest(_)) = self.recv().await? else {
            bail!("{} sent an unexpected packet during the hello", self.peer());
        };

        self.send(HelloResponse {
            protocol_version: PROTOCOL_VERSION,
            host: host.clone(),
        })
        .await?;

        match self.recv().await? {
            Some(Packet::HelloFinished(finished)) if finished.compatible => Ok(()),
            Some(Packet::HelloFinished(_)) => {
                Err(eyre!("{} is incompatible with this server", self.peer()))
            }
            _ => Err(eyre!(
                "{} sent an unexpected packet during the hello",
                self.peer()
            )),
        }
    }
}
