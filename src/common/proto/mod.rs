pub mod envelope;

#[cfg(test)]
mod test;

pub use coeus_proto_macros::{Packet, packet_enum};

pub const PROTOCOL_VERSION: u8 = 2;

pub mod types;
pub use types::*;

#[packet_enum]
#[derive(Debug)]
pub enum Packet {
    #[packet(id = 0x0000, type = Error)]
    Error,

    #[packet(id = 0x0001, type = HelloRequest)]
    HelloRequest,
    #[packet(id = 0x0002, type = HelloResponse)]
    HelloResponse,
    #[packet(id = 0x0003, type = HelloFinished)]
    HelloFinished,

    #[packet(id = 0x0100, type = DeployRequest)]
    DeployRequest,
    #[packet(id = 0x0101, type = DeployAccepted)]
    DeployAccepted,
    #[packet(id = 0x0102, type = DeployRejected)]
    DeployRejected,
    #[packet(id = 0x0103, type = CancelRequest)]
    CancelRequest,
    #[packet(id = 0x0104, type = CancelResponse)]
    CancelResponse,
    #[packet(id = 0x0105, type = ResolveRequest)]
    ResolveRequest,
    #[packet(id = 0x0106, type = ResolveResponse)]
    ResolveResponse,

    #[packet(id = 0x0110, type = UploadBegin)]
    UploadBegin,
    #[packet(id = 0x0111, type = UploadReady)]
    UploadReady,
    #[packet(id = 0x0112, type = UploadChunk)]
    UploadChunk,
    #[packet(id = 0x0113, type = UploadDone)]
    UploadDone,

    #[packet(id = 0x0200, type = WatchRequest)]
    WatchRequest,
    #[packet(id = 0x0201, type = MachineUpdate)]
    MachineUpdate,
    #[packet(id = 0x0202, type = AttachRequest)]
    AttachRequest,
    #[packet(id = 0x0203, type = AttachStarted)]
    AttachStarted,
    #[packet(id = 0x0204, type = AttachBatch)]
    AttachBatch,
    #[packet(id = 0x0205, type = AttachEnded)]
    AttachEnded,
    #[packet(id = 0x0206, type = HistoryRequest)]
    HistoryRequest,
    #[packet(id = 0x0207, type = HistoryResponse)]
    HistoryResponse,

    #[packet(id = 0x0300, type = GenerationsRequest)]
    GenerationsRequest,
    #[packet(id = 0x0301, type = GenerationsResponse)]
    GenerationsResponse,
    #[packet(id = 0x0302, type = RebootRequest)]
    RebootRequest,
    #[packet(id = 0x0303, type = Ack)]
    Ack,
}
