//! the pull request through `gh`, found by its head branch, so a run that
//! died after `gh pr create` finds the one it opened

use std::path::Path;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PullRequest {
    pub number: u64,
    pub url: String,
    pub draft: bool,
    pub state: PullState,
    pub body: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PullState {
    Open,
    Merged,
    /// closed without merging: reopened only when the person says so
    Closed,
}

impl PullRequest {
    /// `gh pr list --head <branch> --state all --json …`, the newest
    pub async fn find(clone: &Path, branch: &str) -> eyre::Result<Option<Self>> {
        todo!()
    }

    /// `gh pr create --draft --base <base> --head <branch> --title … --body …`
    pub async fn create_draft(
        clone: &Path,
        branch: &str,
        base: &str,
        title: &str,
        body: &str,
    ) -> eyre::Result<Self> {
        todo!()
    }

    /// `gh pr edit --body` with coeus's section replaced, only when it differs
    pub async fn edit_body_section(&mut self, clone: &Path, section: &str) -> eyre::Result<()> {
        todo!()
    }

    /// `gh pr ready`, unless it is ready already
    pub async fn ready(&mut self, clone: &Path) -> eyre::Result<()> {
        todo!()
    }
}
