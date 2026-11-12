//! the enrollment lives on `coeus/bootstrap/<host>`, in a private clone under
//! the bootstrap's state directory (`git clone --reference-if-able <the
//! person's checkout> <its origin>`), so the person's checkout, its branch,
//! its uncommitted work and its jj state are never touched.
//!
//! the branch is the bootstrap's durable record, two commits on a freshly
//! fetched `origin/<default>`:
//!
//! ```text
//! <the contract's enroll subject>
//!
//! Coeus-Bootstrap: enroll <host>
//!
//! <the contract's rekey subject>       ← always on top, derived: dropped
//!                                         and made again after a rebase
//! Coeus-Bootstrap: rekey <host>
//! ```
//!
//! a run on this admin or another one reads what was done from the branch.

pub mod body;
mod clone;
mod forge;
mod git;
mod trailer;

#[cfg(test)]
mod test;

use std::{
    collections::BTreeMap,
    io::Write,
    path::{Path, PathBuf},
    sync::Mutex,
};

use eyre::{Result, WrapErr, bail, eyre};

pub use forge::{GhError, PullRequest, PullState};
use git::Git;
use trailer::{LOG_FORMAT, Trailer, parse_log};

use crate::{
    bootstrap::{contract::RepoPath, state::StateDir},
    common::model::{CommitSha, Hostname},
};

pub struct Branch {
    host: Hostname,
    clone: PathBuf,
    /// origin's default branch, from `refs/remotes/origin/HEAD`
    base: String,
    /// the branch's head on origin when the run fetched it: the push's lease
    seen: Mutex<Option<CommitSha>>,
}

/// the enroll commit, as the branch shows it
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Enrolled {
    pub commit: CommitSha,
    pub subject: String,
    /// the paths it added
    pub files: Vec<RepoPath>,
}

/// how moving the branch onto the default branch went
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rebased {
    Clean,
    /// a commit did not apply; the rebase was aborted and the branch is as
    /// it was
    Conflict,
}

impl Branch {
    pub fn name(host: &Hostname) -> String {
        format!("coeus/bootstrap/{host}")
    }

    /// the clone, made or repaired from whatever is there:
    ///
    /// - no clone at [StateDir::clone_dir], or a half-made one: cloned again
    /// - the branch only on origin (another admin started it): tracked
    /// - no branch anywhere: a new one on a freshly fetched `origin/<base>`
    /// - a stale `index.lock`: removed, since the bootstrap's flock proves no
    ///   git of ours runs there
    ///
    /// `base` is the branch of origin the enrollment goes onto, origin's
    /// default branch when `None`
    pub async fn open(
        state_dir: &StateDir,
        person_checkout: &Path,
        host: &Hostname,
        base: Option<&str>,
    ) -> Result<Self> {
        let dir = state_dir.clone_dir();
        let git = clone::ensure(&dir, person_checkout).await?;
        clone::repair(&git).await?;
        git.run(&["fetch", "--quiet", "--prune", "origin"]).await?;
        let base = match base {
            Some(base) => base.to_owned(),
            None => clone::base(&git).await?,
        };
        if git
            .resolve(&format!("refs/remotes/origin/{base}"))
            .await?
            .is_none()
        {
            bail!("origin has no branch {base} to put the enrollment on");
        }

        let name = Self::name(host);
        let remote = git.resolve(&format!("refs/remotes/origin/{name}")).await?;
        clone::check_out(&git, &name, &base, remote).await?;

        Ok(Self {
            host: host.clone(),
            clone: dir,
            base,
            seen: Mutex::new(remote),
        })
    }

    pub fn clone_dir(&self) -> &Path {
        &self.clone
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    fn git(&self) -> Git {
        Git::at(&self.clone)
    }

    fn onto(&self) -> String {
        format!("origin/{}", self.base)
    }

    pub async fn head(&self) -> Result<CommitSha> {
        self.git()
            .resolve("HEAD")
            .await?
            .ok_or_else(|| eyre!("the clone at {} has no head", self.clone.display()))
    }

    /// the branch's own commits, newest first
    async fn commits(&self) -> Result<Vec<trailer::Marked>> {
        let range = format!("{}..HEAD", self.onto());
        let log = self.git().run(&["log", LOG_FORMAT, &range]).await?;
        parse_log(&log, &self.host)
    }

    /// the enroll commit, if the branch has one
    pub async fn enrolled(&self) -> Result<Option<Enrolled>> {
        let commits = self.commits().await?;
        let Some(enroll) = commits
            .into_iter()
            .find(|commit| commit.is(Trailer::Enroll))
        else {
            return Ok(None);
        };
        let sha = enroll.commit.to_string();
        let added = [
            "diff-tree",
            "-r",
            "-z",
            "--no-commit-id",
            "--name-only",
            "--diff-filter=A",
        ];
        let files = self
            .git()
            .paths(&[&added[..], &[sha.as_str()]].concat())
            .await?
            .into_iter()
            .map(RepoPath::try_from)
            .collect::<Result<_>>()?;
        Ok(Some(Enrolled {
            commit: enroll.commit,
            subject: enroll.subject,
            files,
        }))
    }

    /// the declared paths that already exist on `origin/<base>`: the machine
    /// is enrolled there
    pub async fn on_base(&self, paths: &[&RepoPath]) -> Result<Vec<RepoPath>> {
        if paths.is_empty() {
            return Ok(Vec::new());
        }
        let onto = self.onto();
        let mut args = vec!["ls-tree", "-r", "-z", "--name-only", onto.as_str(), "--"];
        args.extend(paths.iter().map(|path| path.as_str()));
        let present = self.git().paths(&args).await?;
        Ok(paths
            .iter()
            .filter(|path| present.iter().any(|found| found == path.as_str()))
            .map(|path| (*path).clone())
            .collect())
    }

    pub fn path(&self, path: &RepoPath) -> PathBuf {
        self.clone.join(path.as_str())
    }

    /// where `path` is written in the clone, its directories made. a symlink
    /// along the way, committed or not, would send the write outside the
    /// clone, so one is refused
    pub fn place(&self, path: &RepoPath) -> Result<PathBuf> {
        let mut dir = self.clone.clone();
        let parts: Vec<&str> = path.as_str().split('/').collect();
        for part in &parts[..parts.len() - 1] {
            dir.push(part);
            let refused = match dir.symlink_metadata() {
                Ok(found) if found.file_type().is_symlink() => "a symlink",
                Ok(found) if !found.is_dir() => "a file",
                Ok(_) => continue,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    std::fs::create_dir(&dir)
                        .wrap_err_with(|| format!("failed to make {}", dir.display()))?;
                    continue;
                }
                Err(error) => return Err(error.into()),
            };
            bail!("{path} goes through {refused} at {}", dir.display());
        }
        Ok(self.path(path))
    }

    /// writes the files that are absent, never edits one that exists, and
    /// stages every declared path; returns the ones it wrote
    pub async fn write(&self, files: &BTreeMap<RepoPath, String>) -> Result<Vec<RepoPath>> {
        let mut written = Vec::new();
        for (path, text) in files {
            let target = self.place(path)?;
            // a temporary sibling renamed into place: a run killed midway
            // leaves no half-written file that the next run would keep
            let partial = target.with_file_name(format!(
                ".{}.coeus-partial",
                target.file_name().unwrap_or_default().to_string_lossy()
            ));
            if partial.symlink_metadata().is_ok() {
                std::fs::remove_file(&partial)?;
            }
            if target.symlink_metadata().is_ok() {
                continue;
            }
            // never through whatever appeared at the partial's name since
            let mut file = std::fs::File::create_new(&partial)?;
            file.write_all(text.as_bytes())?;
            file.sync_all()?;
            std::fs::rename(&partial, &target)
                .wrap_err_with(|| format!("failed to write {path}"))?;
            written.push(path.clone());
        }
        self.add(&files.keys().collect::<Vec<_>>()).await?;
        Ok(written)
    }

    /// stages paths the contract declared, even ones the repository ignores
    async fn add(&self, paths: &[&RepoPath]) -> Result<()> {
        if paths.is_empty() {
            return Ok(());
        }
        let mut args = vec!["--literal-pathspecs", "add", "--force", "--"];
        args.extend(paths.iter().map(|path| path.as_str()));
        self.git().run(&args).await?;
        Ok(())
    }

    /// commits `paths` and nothing else staged, amending the head when asked
    async fn commit(
        &self,
        subject: &str,
        trailer: Trailer,
        paths: &[RepoPath],
        amend: bool,
    ) -> Result<()> {
        if paths.is_empty() {
            bail!("nothing to commit for {subject:?}");
        }
        self.add(&paths.iter().collect::<Vec<_>>()).await?;
        let trailer = trailer.line(&self.host);
        let mut args = vec!["--literal-pathspecs", "commit", "--quiet", "--no-edit"];
        if amend {
            args.push("--amend");
        }
        args.extend(["-m", subject, "-m", &trailer, "--only", "--"]);
        args.extend(paths.iter().map(RepoPath::as_str));
        self.git().run(&args).await?;
        Ok(())
    }

    /// commits exactly `paths` with the enroll trailer
    pub async fn commit_enroll(&self, subject: &str, paths: &[RepoPath]) -> Result<Enrolled> {
        self.commit(subject, Trailer::Enroll, paths, false).await?;
        self.enrolled()
            .await?
            .ok_or_else(|| eyre!("the enroll commit is not on {}", Self::name(&self.host)))
    }

    /// fetches origin; the commits `origin/<base>` has that the branch lacks
    pub async fn behind(&self) -> Result<u32> {
        let git = self.git();
        let refspec = format!("+refs/heads/{0}:refs/remotes/origin/{0}", self.base);
        git.run(&["fetch", "--quiet", "origin", &refspec]).await?;
        let range = format!("HEAD..{}", self.onto());
        let count = git.run(&["rev-list", "--count", &range]).await?;
        count
            .trim()
            .parse()
            .wrap_err_with(|| format!("unexpected commit count {count:?}"))
    }

    /// drops the rekey commit and rebases the rest onto `origin/<base>`. a
    /// conflict aborts the rebase and leaves the branch as it was.
    pub async fn rebase_dropping_rekey(&self) -> Result<Rebased> {
        let git = self.git();
        let onto = self.onto();
        let commits = self.commits().await?;
        let rekeys: Vec<String> = commits
            .iter()
            .filter(|commit| commit.is(Trailer::Rekey))
            .map(|commit| commit.commit.to_string())
            .collect();
        // the branch is the base plus the enroll commit and whatever a person
        // committed after it. commits from before the enroll commit belong to
        // the base it was cut from, which may since have been rewritten, so
        // they are never replayed
        let own: Vec<String> = match commits.iter().find(|commit| commit.is(Trailer::Enroll)) {
            Some(enroll) => {
                let range = format!("{}^..HEAD", enroll.commit);
                git.run(&["rev-list", "--no-merges", &range])
                    .await?
                    .lines()
                    .map(str::to_owned)
                    .collect()
            }
            None => Vec::new(),
        };
        // of those, what `onto` does not have an equal of already
        let range = format!("{onto}...HEAD");
        let replay = [
            "rev-list",
            "--reverse",
            "--no-merges",
            "--cherry-pick",
            "--right-only",
        ];
        let keep: Vec<String> = git
            .run(&[&replay[..], &[range.as_str()]].concat())
            .await?
            .lines()
            .filter(|sha| own.iter().any(|commit| commit == sha))
            .filter(|sha| !rekeys.iter().any(|rekey| rekey == sha))
            .map(str::to_owned)
            .collect();

        if keep.is_empty() {
            git.run(&["reset", "--quiet", "--keep", &onto]).await?;
        } else if !self.rebase(&onto, &keep).await? {
            git.run(&["rebase", "--abort"]).await?;
            return Ok(Rebased::Conflict);
        }

        let on_base = git
            .test(&["merge-base", "--is-ancestor", &onto, "HEAD"])
            .await?;
        let rekeyed = self
            .commits()
            .await?
            .iter()
            .any(|commit| commit.is(Trailer::Rekey));
        if !on_base || rekeyed {
            bail!(
                "the rebase left {} off {onto} or with its rekey commit",
                Self::name(&self.host)
            );
        }
        Ok(Rebased::Clean)
    }

    /// `git rebase -i` with `keep` as its whole todo list; false when it
    /// stopped on a conflict
    async fn rebase(&self, onto: &str, keep: &[String]) -> Result<bool> {
        let git = self.git();
        let todo = self.clone.join(".git").join("coeus-rebase-todo");
        let list: String = keep.iter().map(|sha| format!("pick {sha}\n")).collect();
        std::fs::write(&todo, list)?;

        // git runs the sequence editor through the shell with the todo
        // file's path appended
        let editor = format!("cp {}", shell_quote(&todo.to_string_lossy()));
        let args = [
            // the rekey commit is dropped by leaving it out of the list,
            // which a person's `error` setting would stop the rebase on
            "-c",
            "rebase.missingCommitsCheck=ignore",
            "rebase",
            "--interactive",
            "--empty=drop",
            "--no-autosquash",
            "--no-autostash",
            "--no-update-refs",
            "--no-rerere-autoupdate",
            onto,
        ];
        let env = [
            ("GIT_SEQUENCE_EDITOR", editor.as_str()),
            ("GIT_EDITOR", "true"),
        ];
        let output = git.output(&args, &env).await?;
        std::fs::remove_file(&todo).ok();

        if output.status.success() {
            return Ok(true);
        }
        let stopped = self.clone.join(".git").join("rebase-merge").exists();
        if stopped {
            return Ok(false);
        }
        git::check(&args, &output)?;
        Ok(true)
    }

    /// paths with changes nobody committed
    pub async fn uncommitted(&self) -> Result<Vec<String>> {
        let status = ["status", "--porcelain=v1", "-z", "--untracked-files=all"];
        Ok(git::status_paths(&self.git().run(&status).await?))
    }

    /// `path` as the head commits it, bytes and all
    pub async fn committed(&self, path: &RepoPath) -> Result<Vec<u8>> {
        let object = format!("HEAD:{path}");
        let args = ["cat-file", "blob", object.as_str()];
        let output = self.git().output(&args, &[]).await?;
        git::check(&args, &output)?;
        Ok(output.stdout)
    }

    /// every tracked `.age` file
    pub async fn secrets(&self) -> Result<Vec<RepoPath>> {
        self.git()
            .paths(&["ls-files", "-z"])
            .await?
            .into_iter()
            .filter(|path| path.ends_with(".age"))
            .map(RepoPath::try_from)
            .collect()
    }

    /// the rekeyed files as the top commit, replacing an older rekey commit
    pub async fn commit_rekey(&self, subject: &str, rekeyed: &[RepoPath]) -> Result<CommitSha> {
        let top = self.commits().await?.into_iter().next();
        let amend = top.is_some_and(|top| top.is(Trailer::Rekey));
        self.commit(subject, Trailer::Rekey, rekeyed, amend).await?;
        self.head().await
    }

    /// `git push --force-with-lease` of the head, against the remote head it
    /// saw, since a rebase rewrites; nothing when origin has it already.
    /// whether it pushed.
    pub async fn push(&self) -> Result<bool> {
        let head = self.head().await?;
        let seen = *self.seen.lock().expect("the lease is never poisoned");
        if seen == Some(head) {
            return Ok(false);
        }

        let name = Self::name(&self.host);
        let expected = seen.map(|sha| sha.to_string()).unwrap_or_default();
        let lease = format!("--force-with-lease=refs/heads/{name}:{expected}");
        let refspec = format!("refs/heads/{name}:refs/heads/{name}");
        let args = [
            "push",
            "--quiet",
            "origin",
            lease.as_str(),
            refspec.as_str(),
        ];
        let git = self.git();
        let output = git.output(&args, &[]).await?;

        if !output.status.success() {
            let remote = format!("refs/heads/{name}");
            let listed = git.run(&["ls-remote", "origin", &remote]).await?;
            let now = listed
                .split_whitespace()
                .next()
                .map(str::parse)
                .transpose()?;
            if now != seen {
                bail!(
                    "{name} on origin moved since this run read it ({} → {}): another run \
                     pushed it; running again picks it up",
                    short(seen),
                    short(now),
                );
            }
            git::check(&args, &output)?;
        }

        let pushed = git.resolve(&format!("refs/remotes/origin/{name}")).await?;
        if pushed != Some(head) {
            bail!("pushed {name}, but origin does not show {}", head.short());
        }
        *self.seen.lock().expect("the lease is never poisoned") = Some(head);
        Ok(true)
    }
}

fn short(sha: Option<CommitSha>) -> String {
    sha.map_or_else(|| "nothing".to_owned(), |sha| sha.short())
}

/// `text` as one single-quoted shell word
fn shell_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}
