mod deploy;

pub use deploy::*;

mod job;
pub use job::*;

mod terminal;
pub use terminal::*;

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct IdentityRequest {}
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct IdentityResponse {
    pub identity: crate::activation::DaemonIdentity,
}
impl From<IdentityRequest> for super::Packet {
    fn from(v: IdentityRequest) -> Self {
        Self::IdentityRequest(Box::new(v))
    }
}
impl From<IdentityResponse> for super::Packet {
    fn from(v: IdentityResponse) -> Self {
        Self::IdentityResponse(Box::new(v))
    }
}

mod machine;
pub use machine::*;
