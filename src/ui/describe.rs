use crate::common::model::{Failure, Rejection};

/// the fix macOS asks for before coeus may manage users there
pub fn full_disk_access_fix(launcher: &str) -> String {
    format!("grant {launcher} in System Settings > Privacy & Security > Full Disk Access")
}

/// a rejection as a sentence
pub fn rejection(rejection: &Rejection) -> String {
    match rejection {
        Rejection::Unsupported { action, platform } => {
            format!("{action} is not available on {platform}")
        }
        Rejection::UnknownSnapshot => "the uploaded snapshot is no longer on the machine".into(),
        Rejection::UnknownGeneration(number) => format!("generation {number} does not exist"),
        Rejection::Restarting => {
            "the daemon is restarting into a newer version, retry in a moment".into()
        }
    }
}

/// `in building: builder for 'foo.drv' failed`
pub fn failure(failure: &Failure) -> String {
    format!("in {}: {}", failure.phase, failure.message)
}
