pub mod envelope;

#[cfg(test)]
mod test;

pub use coeus_proto_macros::{Packet, packet_enum};

pub const PROTOCOL_VERSION: u8 = 6;

pub mod types;
pub use types::*;

#[packet_enum]
#[derive(Debug)]
pub enum Packet {
    #[packet(id = 0x0302, type = HelloRequest)]
    HelloRequest,
    #[packet(id = 0x0303, type = HelloResponse)]
    HelloResponse,
    #[packet(id = 0x0208, type = MachineStatusRequest)]
    MachineStatusRequest,
    #[packet(id = 0x0209, type = MachineStatusResponse)]
    MachineStatusResponse,
    #[packet(id = 0x020a, type = SharedTerminalRequest)]
    SharedTerminalRequest,
    #[packet(id = 0x020b, type = SharedTerminalResponse)]
    SharedTerminalResponse,
    #[packet(id = 0x0300, type = IdentityRequest)]
    IdentityRequest,
    #[packet(id = 0x0301, type = IdentityResponse)]
    IdentityResponse,
    #[packet(id = 0x0100, type = DeployRequest)]
    DeployRequest,

    #[packet(id = 0x0101, type = DeployAccepted)]
    DeployAccepted,
    #[packet(id = 0x0200, type = StatusRequest)]
    StatusRequest,
    #[packet(id = 0x0201, type = StatusResponse)]
    StatusResponse,
    #[packet(id = 0x0202, type = LogsRequest)]
    LogsRequest,
    #[packet(id = 0x0203, type = LogsResponse)]
    LogsResponse,
    #[packet(id = 0x0204, type = CancelRequest)]
    CancelRequest,
    #[packet(id = 0x0206, type = TerminalRequest)]
    TerminalRequest,
    #[packet(id = 0x0207, type = TerminalResponse)]
    TerminalResponse,
    #[packet(id = 0x0205, type = RequestRejected)]
    RequestRejected,
}
