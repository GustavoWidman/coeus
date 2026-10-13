use super::Deployer;
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
                let commit = repo
                    .revparse_single(&rev)
                    .or_else(|_| repo.revparse_single(&format!("origin/{rev}")))?
                    .peel_to_commit()?;
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
        let config_path = dir.path().join("server.toml");
        std::fs::write(
            &config_path,
            format!(
                "[settings]\nkey = \"{}\"\nhost = \"127.0.0.1\"\nport = 0\nrepo_url = {:?}\n",
                "42".repeat(32),
                source.to_string_lossy()
            ),
        )?;
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
        Ok(())
    }
}
