use super::Packet;

/// what a connection may ask of a daemon, decided by the key it
/// authenticated with
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Full,
    /// may watch the machine and read its history, generations and logs,
    /// and nothing that makes the daemon do work
    Read,
}

impl Role {
    pub const fn allows(self, packet: &Packet) -> bool {
        match self {
            Self::Full => true,
            // every packet is named, so a new one does not compile until
            // someone decides whether a read-only key may send it
            Self::Read => match packet {
                Packet::WatchRequest(_)
                | Packet::HistoryRequest(_)
                | Packet::GenerationsRequest(_) => true,
                // a drawn attach spawns nix-output-monitor on the daemon
                Packet::AttachRequest(request) => request.terminal.is_none(),
                // resolving fetches git, and catching up can start a job
                Packet::DeployRequest(_)
                | Packet::CancelRequest(_)
                | Packet::ResolveRequest(_)
                | Packet::CatchUpRequest(_)
                | Packet::RebootRequest(_)
                | Packet::UploadBegin(_)
                | Packet::UploadChunk(_)
                | Packet::AttachResize(_) => false,
                // only the hello or a server sends these
                Packet::Error(_)
                | Packet::HelloRequest(_)
                | Packet::HelloResponse(_)
                | Packet::HelloFinished(_)
                | Packet::DeployAccepted(_)
                | Packet::DeployRejected(_)
                | Packet::CancelResponse(_)
                | Packet::ResolveResponse(_)
                | Packet::CatchUpResponse(_)
                | Packet::UploadReady(_)
                | Packet::UploadDone(_)
                | Packet::MachineUpdate(_)
                | Packet::AttachStarted(_)
                | Packet::AttachBatch(_)
                | Packet::AttachEnded(_)
                | Packet::HistoryResponse(_)
                | Packet::AttachScreen(_)
                | Packet::GenerationsResponse(_)
                | Packet::Ack(_) => false,
            },
        }
    }
}
