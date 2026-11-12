use serde::{Deserialize, Serialize};

use crate::common::{model::NarHash, proto::Packet};

/// offer a working tree, serialized as a NAR, for a later `Source::Snapshot`
#[derive(Debug, Serialize, Deserialize, Packet)]
pub struct UploadBegin {
    pub nar_hash: NarHash,
    pub size: u64,
}

/// where to continue: the size when the daemon already has the snapshot
#[derive(Debug, Serialize, Deserialize, Packet)]
pub struct UploadReady {
    pub offset: u64,
}

#[derive(Debug, Serialize, Deserialize, Packet)]
pub struct UploadChunk {
    pub data: Vec<u8>,
}

/// the daemon verified the hash and imported the snapshot
#[derive(Debug, Serialize, Deserialize, Packet)]
pub struct UploadDone {}
