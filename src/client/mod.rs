//! the CLI's side of the protocol: one connection per daemon, typed
//! conversations, and the fleet of them

mod fleet;
mod origin;
mod stream;
mod targets;

#[cfg(test)]
mod test;

use colored::Colorize;
use eyre::{Result, bail, eyre};
use log::debug;
use tokio::{net::lookup_host, time::timeout};

pub use fleet::{Fleet, FleetEvent, Link};
pub use origin::detect_origin;
pub use stream::{Attachment, AttachmentEvent};
pub use targets::Targets;

use crate::{
    common::{
        model::{
            Admission, CancelReport, Generation, HostInfo, JobSelector, JobSummary, MachineView,
            Order, Pinned, Rejection,
        },
        proto::{
            AttachRequest, CancelRequest, DeployRequest, GenerationsRequest, HistoryRequest,
            Packet, RebootRequest, ResolveRequest, WatchRequest,
        },
        stream::EncryptedStream,
    },
    config::{ClientSettings, Endpoint, Key},
};

/// one connection to one daemon. every method is one conversation; the
/// streaming ones consume the connection.
pub struct CoeusClient {
    stream: EncryptedStream,
    host: HostInfo,
}

impl CoeusClient {
    pub async fn connect(
        settings: &ClientSettings,
        endpoint: &Endpoint,
        key: &Key,
    ) -> Result<Self> {
        let attempt = async {
            let address = lookup_host((endpoint.host.as_str(), endpoint.port))
                .await?
                .next()
                .ok_or_else(|| eyre!("{} does not resolve", endpoint.host))?;

            EncryptedStream::connect(address, key.as_bytes()).await
        };

        let (stream, hello) = timeout(settings.connect_timeout, attempt)
            .await
            .map_err(|_| eyre!("no answer within {:?}", settings.connect_timeout))??;

        debug!("connected to {}", hello.host.hostname.to_string().magenta());
        Ok(Self {
            stream,
            host: hello.host,
        })
    }

    /// the machine on the other end, as its daemon introduced itself
    pub const fn host(&self) -> &HostInfo {
        &self.host
    }

    pub async fn deploy(&mut self, order: Order) -> Result<Result<Admission, Rejection>> {
        match self.ask(DeployRequest { order }).await? {
            Packet::DeployAccepted(accepted) => Ok(Ok(accepted.admission)),
            Packet::DeployRejected(rejected) => Ok(Err(rejected.rejection)),
            other => unexpected(other),
        }
    }

    pub async fn cancel(&mut self, job: JobSelector) -> Result<CancelReport> {
        match self.ask(CancelRequest { job }).await? {
            Packet::CancelResponse(response) => Ok(response.report),
            other => unexpected(other),
        }
    }

    pub async fn resolve(&mut self, git_ref: &str) -> Result<Pinned> {
        let request = ResolveRequest {
            git_ref: git_ref.to_owned(),
        };
        match self.ask(request).await? {
            Packet::ResolveResponse(response) => Ok(response.pinned),
            other => unexpected(other),
        }
    }

    /// newest first
    pub async fn history(&mut self, limit: u32) -> Result<Vec<JobSummary>> {
        match self.ask(HistoryRequest { limit }).await? {
            Packet::HistoryResponse(response) => Ok(response.jobs),
            other => unexpected(other),
        }
    }

    /// newest first
    pub async fn generations(&mut self) -> Result<Vec<Generation>> {
        match self.ask(GenerationsRequest {}).await? {
            Packet::GenerationsResponse(response) => Ok(response.generations),
            other => unexpected(other),
        }
    }

    pub async fn reboot(&mut self) -> Result<()> {
        match self.ask(RebootRequest {}).await? {
            Packet::Ack(_) => Ok(()),
            other => unexpected(other),
        }
    }

    /// the machine's view right now
    pub async fn view(&mut self) -> Result<MachineView> {
        match self.ask(WatchRequest { follow: false }).await? {
            Packet::MachineUpdate(update) => Ok(update.view),
            other => unexpected(other),
        }
    }

    /// the machine's view, every time it changes, until the connection drops
    pub async fn watch(mut self) -> Result<stream::Views> {
        self.stream.send(WatchRequest { follow: true }).await?;
        Ok(stream::Views::new(self.stream))
    }

    /// a job's log from `from`, live until the job finishes
    pub async fn attach(mut self, job: JobSelector, from: u64) -> Result<Attachment> {
        self.stream.send(AttachRequest { job, from }).await?;
        Ok(Attachment::new(self.stream))
    }

    async fn ask(&mut self, request: impl Into<Packet>) -> Result<Packet> {
        self.stream.send(request).await?;
        match self.stream.recv().await? {
            Some(Packet::Error(error)) => {
                return Err(eyre!("{}: {}", self.host.hostname, error.message));
            }
            Some(packet) => Ok(packet),
            None => return Err(eyre!("{} closed the connection", self.host.hostname)),
        }
    }
}

fn unexpected<T>(packet: Packet) -> Result<T> {
    Err(eyre!(
        "the daemon answered with an unexpected packet {:#06x}",
        packet.kind()
    ))
}
