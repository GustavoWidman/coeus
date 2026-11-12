//! the job's progress on stdout, for `coeus bootstrap` reading it over ssh:
//!
//! ```text
//! @job <JobSummary json>        once, first
//! @coeus {"at":…,"event":…}     the job log's own lines, as they are
//! @nix <internal-json>
//! @at <offset>                  after each, where a follower resumes
//! ```
//!
//! a closed stdout (the ssh connection gone) just ends it; the runner does
//! not care.

use crate::job::JobDir;

/// until the log holds the finishing event
pub async fn stream(dir: &JobDir, from: u64) -> eyre::Result<()> {
    todo!()
}
