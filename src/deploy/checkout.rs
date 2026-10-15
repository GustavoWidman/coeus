use super::Deployer;
use eyre::WrapErr;
use git2::{
    Cred, CredentialType, FetchOptions, RemoteCallbacks, Repository, build::CheckoutBuilder,
};
use log::debug;
use tempfile::TempDir;

pub(super) struct Checkout {
    pub directory: TempDir,
    pub commit: String,
}

fn create_remote_with_local_config<'repo>(
    repo: &'repo Repository,
    name: &str,
    url: &str,
) -> std::result::Result<git2::Remote<'repo>, git2::Error> {
    let original_config = repo.config()?;
    let local_config = git2::Config::open(&repo.path().join("config"))?;

    repo.set_config(&local_config)?;
    let remote = repo.remote(name, url);
    repo.set_config(&original_config)?;

    remote
}

/// Resolve only objects present in the configured repository clone. An exact OID
/// is never reinterpreted as a branch name, nor fetched from another repository.
fn resolve_commit<'repo>(repo: &'repo Repository, rev: &str) -> eyre::Result<git2::Commit<'repo>> {
    if rev.len() == 40 && rev.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        let oid = git2::Oid::from_str(rev)?;
        return repo.find_commit(oid).wrap_err_with(|| {
            format!(
                "requested commit {rev} is not available as a commit in the configured repository clone; ensure it is published and reachable from that repository's advertised refs (local-only commits are not cloned); no branch fallback or additional fetch was attempted"
            )
        });
    }

    match repo.revparse_single(rev) {
        Ok(object) => object
            .peel_to_commit()
            .wrap_err_with(|| format!("requested revision {rev:?} does not resolve to a commit")),
        Err(original) if original.code() == git2::ErrorCode::NotFound => {
            // Non-default branches are remote-tracking refs after cloning. Look up
            // an exact branch ref, not a second arbitrary revspec that masks the
            // first failure (or changes the meaning of an unavailable commit ID).
            let remote_ref = format!("refs/remotes/origin/{rev}");
            match repo.find_reference(&remote_ref) {
                Ok(reference) => reference.peel_to_commit().wrap_err_with(|| {
                    format!("remote branch {remote_ref:?} does not resolve to a commit")
                }),
                Err(remote_error) => Err(original).wrap_err_with(|| {
                    format!(
                        "requested revision {rev:?} is not available in the configured repository clone; remote branch {remote_ref:?} also could not be resolved: {remote_error}"
                    )
                }),
            }
        }
        Err(error) => {
            Err(error).wrap_err_with(|| format!("could not resolve requested revision {rev:?}"))
        }
    }
}

impl Deployer {
    pub(super) async fn checkout(
        &self,
        rev: String,
        cancellation: tokio_util::sync::CancellationToken,
    ) -> eyre::Result<Checkout> {
        let dir = tempfile::tempdir()?;
        debug!("preparing temporary repository checkout");

        let commit = tokio::task::spawn_blocking({
            let url = self.config.repo_url.clone();
            let path = dir.path().to_path_buf();

            move || -> eyre::Result<String> {
                if cancellation.is_cancelled() {
                    eyre::bail!("checkout cancelled before clone");
                }
                let transfer_cancellation = cancellation.clone();
                let mut callbacks = RemoteCallbacks::new();
                callbacks.transfer_progress(move |_| !transfer_cancellation.is_cancelled());
                callbacks.credentials(|_url, username, allowed_types| {
                    if allowed_types.contains(CredentialType::SSH_KEY) {
                        Cred::ssh_key_from_agent(username.unwrap_or("git"))
                    } else if allowed_types.contains(CredentialType::USERNAME) {
                        Cred::username(username.unwrap_or("git"))
                    } else {
                        Cred::default()
                    }
                });

                let mut fetch_options = FetchOptions::new();
                fetch_options.remote_callbacks(callbacks);

                let mut builder = git2::build::RepoBuilder::new();
                builder.fetch_options(fetch_options);
                builder.remote_create(create_remote_with_local_config);
                let repo = builder.clone(&url, &path)?;

                if cancellation.is_cancelled() {
                    eyre::bail!("checkout cancelled after fetch");
                }
                let commit = resolve_commit(&repo, &rev)?;
                repo.checkout_tree(commit.as_object(), Some(CheckoutBuilder::new().force()))?;
                // checkout_tree alone leaves HEAD at the clone's original revision.
                repo.set_head_detached(commit.id())?;
                if cancellation.is_cancelled() {
                    eyre::bail!("checkout cancelled after checkout");
                }
                Ok(commit.id().to_string())
            }
        })
        .await??;

        Ok(Checkout {
            directory: dir,
            commit,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_remote_without_composite_url_rewrite() {
        let repo_dir = tempfile::tempdir().unwrap();
        let repo = Repository::init(repo_dir.path()).unwrap();
        let global_config = repo_dir.path().join("global.gitconfig");
        std::fs::write(
            &global_config,
            "[init]\n\tdefaultBranch = main\n[url \"git@example.com:\"]\n\tinsteadOf = https://example.com/\n",
        )
        .unwrap();

        let mut config = git2::Config::new().unwrap();
        config
            .add_file(&global_config, git2::ConfigLevel::Global, false)
            .unwrap();
        config
            .add_file(&repo.path().join("config"), git2::ConfigLevel::Local, false)
            .unwrap();
        repo.set_config(&config).unwrap();
        repo.set_head("refs/heads/main").unwrap();

        let url = "https://example.com/owner/repo.git";
        let remote = create_remote_with_local_config(&repo, "origin", url).unwrap();

        assert_eq!(remote.url(), Ok(url));
        assert_eq!(
            repo.config()
                .unwrap()
                .get_string("url.git@example.com:.insteadOf"),
            Ok("https://example.com/".to_owned())
        );
        assert!(
            repo.config()
                .unwrap()
                .get_string(&format!("url.{url}.insteadOf"))
                .is_err()
        );
        assert!(repo.is_empty().unwrap());
    }

    fn fixture_commit(repo: &Repository) -> git2::Oid {
        let signature = git2::Signature::now("fixture", "fixture@example.invalid").unwrap();
        let tree = repo
            .find_tree(repo.index().unwrap().write_tree().unwrap())
            .unwrap();
        repo.commit(Some("HEAD"), &signature, &signature, "fixture", &tree, &[])
            .unwrap()
    }

    #[test]
    fn resolves_exact_commit_and_remote_branch_without_changing_revision() {
        let dir = tempfile::tempdir().unwrap();
        let repo = Repository::init(dir.path()).unwrap();
        let oid = fixture_commit(&repo);
        repo.reference("refs/remotes/origin/release", oid, false, "fixture")
            .unwrap();
        assert_eq!(resolve_commit(&repo, &oid.to_string()).unwrap().id(), oid);
        assert_eq!(resolve_commit(&repo, "release").unwrap().id(), oid);
        assert_eq!(resolve_commit(&repo, "origin/release").unwrap().id(), oid);
        assert_eq!(resolve_commit(&repo, "HEAD").unwrap().id(), oid);
    }

    #[test]
    fn missing_exact_sha_is_not_reinterpreted_as_a_remote_branch() {
        let dir = tempfile::tempdir().unwrap();
        let repo = Repository::init(dir.path()).unwrap();
        let oid = fixture_commit(&repo);
        let missing = "0123456789012345678901234567890123456789";
        // Deliberate trap: the old origin/<SHA> fallback deploys a different
        // commit if this remote branch exists. Exact SHA selection must fail.
        repo.reference(
            &format!("refs/remotes/origin/{missing}"),
            oid,
            false,
            "fixture",
        )
        .unwrap();
        let error = resolve_commit(&repo, missing).unwrap_err();
        let message = format!("{error:#}");
        assert!(message.contains(&format!("requested commit {missing}")));
        assert!(message.contains("local-only commits are not cloned"));
        assert!(message.contains("no branch fallback or additional fetch"));
        assert!(!message.contains(&format!("origin/{missing}")));
        assert_eq!(
            error.downcast_ref::<git2::Error>().unwrap().code(),
            git2::ErrorCode::NotFound
        );
        assert_eq!(repo.head().unwrap().target(), Some(oid));
    }

    #[test]
    fn missing_branch_preserves_original_error_and_invalid_revspec_is_not_retried() {
        let dir = tempfile::tempdir().unwrap();
        let repo = Repository::init(dir.path()).unwrap();
        fixture_commit(&repo);
        let original = repo.revparse_single("unpublished").unwrap_err();
        let error = resolve_commit(&repo, "unpublished").unwrap_err();
        let message = format!("{error:#}");
        assert!(message.contains("requested revision \"unpublished\""));
        assert!(message.contains("refs/remotes/origin/unpublished"));
        assert_eq!(
            error.downcast_ref::<git2::Error>().unwrap().message(),
            original.message()
        );
        let invalid = resolve_commit(&repo, "HEAD@{").unwrap_err();
        assert!(format!("{invalid:#}").contains("could not resolve requested revision"));
        assert!(!format!("{invalid:#}").contains("remote branch"));
    }

    #[test]
    fn exact_noncommit_object_is_rejected_instead_of_peeling_to_another_commit() {
        let dir = tempfile::tempdir().unwrap();
        let repo = Repository::init(dir.path()).unwrap();
        let oid = fixture_commit(&repo);
        let tree = repo.find_commit(oid).unwrap().tree_id();
        let error = resolve_commit(&repo, &tree.to_string()).unwrap_err();
        assert!(format!("{error:#}").contains("not available as a commit"));
    }

    #[tokio::test]
    async fn local_checkout_detaches_head_at_requested_commit() -> eyre::Result<()> {
        let dir = tempfile::tempdir()?;
        let source = dir.path().join("source");
        let repo = Repository::init(&source)?;
        let signature = git2::Signature::now("fixture", "fixture@example.invalid")?;
        std::fs::write(source.join("marker"), "first")?;
        let mut index = repo.index()?;
        index.add_path(std::path::Path::new("marker"))?;
        index.write()?;
        let first = repo.commit(
            Some("HEAD"),
            &signature,
            &signature,
            "first",
            &repo.find_tree(index.write_tree()?)?,
            &[],
        )?;
        std::fs::write(source.join("marker"), "second")?;
        index.add_path(std::path::Path::new("marker"))?;
        index.write()?;
        repo.commit(
            Some("HEAD"),
            &signature,
            &signature,
            "second",
            &repo.find_tree(index.write_tree()?)?,
            &[&repo.find_commit(first)?],
        )?;
        repo.reference("refs/heads/release", first, false, "fixture")?;
        // An unreferenced object exists only in the source repository, like an
        // unpublished local commit. file:// uses transport cloning rather than
        // the local-path optimization that copies even unreferenced objects.
        let unpublished = repo.commit(
            None,
            &signature,
            &signature,
            "unpublished",
            &repo.find_tree(index.write_tree()?)?,
            &[&repo.find_commit(first)?],
        )?;
        let config_path = dir.path().join("server.toml");
        std::fs::write(
            &config_path,
            format!(
                "[settings]\nkey = \"{}\"\nhost = \"127.0.0.1\"\nport = 0\nrepo_url = {:?}\n",
                "42".repeat(32),
                format!("file://{}", source.display())
            ),
        )?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&config_path, std::fs::Permissions::from_mode(0o600))?;
        }
        let config = crate::config::server_config(config_path)?;
        let deployer = Deployer::new(config).await?;
        // No Deployer::deploy call: clone only a local throwaway fixture; no credentials.
        let checkout = tokio::time::timeout(
            std::time::Duration::from_secs(3),
            deployer.checkout(
                first.to_string(),
                tokio_util::sync::CancellationToken::new(),
            ),
        )
        .await??;
        assert_eq!(checkout.commit, first.to_string());
        assert_eq!(
            std::fs::read_to_string(checkout.directory.path().join("marker"))?,
            "first"
        );
        let cloned = Repository::open(checkout.directory.path())?;
        assert!(cloned.head_detached()?);
        assert_eq!(cloned.head()?.target(), Some(first));
        let branch = tokio::time::timeout(
            std::time::Duration::from_secs(3),
            deployer.checkout(
                "release".to_owned(),
                tokio_util::sync::CancellationToken::new(),
            ),
        )
        .await??;
        assert_eq!(branch.commit, first.to_string());
        let missing = tokio::time::timeout(
            std::time::Duration::from_secs(3),
            deployer.checkout(
                unpublished.to_string(),
                tokio_util::sync::CancellationToken::new(),
            ),
        )
        .await?
        .err()
        .expect("unpublished commit must not be cloned");
        assert!(format!("{missing:#}").contains("local-only commits are not cloned"));
        Ok(())
    }
}
