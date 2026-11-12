mod body;
mod forge;

pub(super) fn path(path: &str) -> super::RepoPath {
    path.parse().unwrap()
}
