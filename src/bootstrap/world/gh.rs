//! the forge through `gh`

use std::path::Path;

use eyre::Result;

use super::Forge;
use crate::bootstrap::branch::PullRequest;

pub struct Gh;

impl Forge for Gh {
    async fn find(&self, clone: &Path, branch: &str) -> Result<Option<PullRequest>> {
        PullRequest::find(clone, branch).await
    }

    async fn create_draft(
        &self,
        clone: &Path,
        branch: &str,
        base: &str,
        title: &str,
        body: &str,
    ) -> Result<PullRequest> {
        PullRequest::create_draft(clone, branch, base, title, body).await
    }

    async fn edit_body_section(
        &self,
        clone: &Path,
        pull: &mut PullRequest,
        section: &str,
    ) -> Result<()> {
        pull.edit_body_section(clone, section).await
    }

    async fn ready(&self, clone: &Path, pull: &mut PullRequest) -> Result<()> {
        pull.ready(clone).await
    }
}
