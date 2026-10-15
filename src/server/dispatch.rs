use super::CoeusServer;
use crate::common::proto::*;
use eyre::{Result, bail};

impl CoeusServer {
    pub(super) async fn dispatch(&self, packet: Packet) -> Packet {
        match self.handle_request(packet).await {
            Ok(response) => response,
            Err(error) => RequestRejected {
                message: crate::jobs::bounded_error(&error),
            }
            .into(),
        }
    }
    async fn handle_request(&self, packet: Packet) -> Result<Packet> {
        Ok(match packet {
            Packet::IdentityRequest(_) => IdentityResponse {
                identity: tokio::task::spawn_blocking(crate::activation::daemon_identity).await??,
            }
            .into(),
            Packet::DeployRequest(request) => {
                let job_id = self.jobs.accept(*request).await?;
                DeployAccepted {
                    job_id,
                    message: "accepted; inspect status for current phase (unfinished jobs never replay on restart)".into(),
                }
                .into()
            }
            Packet::StatusRequest(request) => self
                .jobs
                .status_page(request.job_id, request.after)
                .await?
                .into(),
            Packet::LogsRequest(request) => {
                self.jobs.logs(request.job_id, request.after).await?.into()
            }
            Packet::TerminalRequest(request) => self.jobs.terminal(*request).await?.into(),
            Packet::CancelRequest(request) => StatusResponse {
                jobs: vec![self.jobs.cancel(request.job_id).await?],
                next_after: None,
            }
            .into(),
            _ => {
                bail!("client sent a response packet");
            }
        })
    }
}
