mod body;
mod commits;
mod forge;
mod open;
mod push;
mod trailer;

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    process::Command,
};

use tempfile::TempDir;

use super::Branch;
use crate::{bootstrap::state::StateDir, common::model::Hostname};

/// a bare origin on `main`, the person's checkout of it, and state homes for
/// admins, all in one temporary directory
pub(super) struct Fleet {
    _dir: TempDir,
    pub root: PathBuf,
    pub origin: PathBuf,
    pub checkout: PathBuf,
}

/// `git` as a test's own hands, with an identity of its own and no signing
pub(super) fn git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.name=test", "-c", "user.email=test@example.com"])
        .args(["-c", "commit.gpgsign=false"])
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

pub(super) fn host() -> Hostname {
    "nova".parse().unwrap()
}

pub(super) fn path(path: &str) -> super::RepoPath {
    path.parse().unwrap()
}

pub(super) fn files(entries: &[(&str, &str)]) -> BTreeMap<super::RepoPath, String> {
    entries
        .iter()
        .map(|(name, text)| (path(name), (*text).to_owned()))
        .collect()
}

impl Fleet {
    pub fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let origin = root.join("origin.git");
        let checkout = root.join("checkout");

        git(
            &root,
            &["init", "--quiet", "--bare", "-b", "main", "origin.git"],
        );
        git(&root, &["clone", "--quiet", "origin.git", "checkout"]);
        let fleet = Self {
            _dir: dir,
            root,
            origin,
            checkout,
        };
        fleet.commit_on_main(&[
            ("readme.md", "the fleet\n"),
            ("secrets/cache.age", "sealed\n"),
        ]);
        fleet
    }

    /// a commit on origin's `main`, as another person merging something
    pub fn commit_on_main(&self, entries: &[(&str, &str)]) {
        self.commit_on("main", entries);
    }

    /// a commit on origin's `branch`, made in the checkout on that branch
    pub fn commit_on(&self, branch: &str, entries: &[(&str, &str)]) {
        if git(&self.checkout, &["branch", "--show-current"]) != branch {
            git(&self.checkout, &["checkout", "--quiet", branch]);
        }
        for (name, text) in entries {
            let file = self.checkout.join(name);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, text).unwrap();
        }
        git(&self.checkout, &["add", "--all"]);
        git(
            &self.checkout,
            &["commit", "--quiet", "-m", &format!("chore: move {branch}")],
        );
        git(
            &self.checkout,
            &["push", "--quiet", "origin", &format!("HEAD:{branch}")],
        );
    }

    pub fn state(&self, admin: &str) -> StateDir {
        StateDir::under(&self.root.join(admin), &host())
    }

    /// the branch as `admin` opens it, its clone committing as the test
    pub async fn open(&self, admin: &str) -> Branch {
        let branch = Branch::open(&self.state(admin), &self.checkout, &host(), None)
            .await
            .unwrap();
        let clone = branch.clone_dir();
        git(clone, &["config", "user.name", admin]);
        git(clone, &["config", "user.email", "admin@example.com"]);
        git(clone, &["config", "commit.gpgsign", "false"]);
        branch
    }

    /// the branch's head on origin
    pub fn on_origin(&self) -> Option<String> {
        let output = Command::new("git")
            .arg("-C")
            .arg(&self.origin)
            .args([
                "rev-parse",
                "--verify",
                "--quiet",
                "refs/heads/coeus/bootstrap/nova",
            ])
            .output()
            .unwrap();
        output
            .status
            .success()
            .then(|| String::from_utf8(output.stdout).unwrap().trim().to_owned())
    }
}

/// enrolled: two files and a secret committed with the enroll trailer
pub(super) async fn enroll(branch: &Branch) -> super::Enrolled {
    let files = files(&[
        ("hosts/nova/flake.nix", "{ }\n"),
        ("hosts/nova/host.pub", "ssh-ed25519 AAAA\n"),
    ]);
    branch.write(&files).await.unwrap();
    std::fs::write(branch.path(&path("hosts/nova/password.age")), "sealed\n").unwrap();

    let mut paths: Vec<_> = files.into_keys().collect();
    paths.push(path("hosts/nova/password.age"));
    branch
        .commit_enroll("feat(nova): join the fleet", &paths)
        .await
        .unwrap()
}

/// the subjects of the branch's own commits, oldest first
pub(super) fn subjects(branch: &Branch) -> Vec<String> {
    let range = format!("origin/{}..HEAD", branch.base());
    let log = git(
        branch.clone_dir(),
        &["log", "--reverse", "--format=%s", &range],
    );
    log.lines().map(str::to_owned).collect()
}
