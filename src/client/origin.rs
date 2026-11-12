use crate::common::model::Origin;

/// who is ordering: a GitHub Actions run when its environment says so,
/// otherwise the user at this machine
pub fn detect_origin() -> Origin {
    let var = |name: &str| std::env::var(name).ok().filter(|value| !value.is_empty());

    if var("GITHUB_ACTIONS").as_deref() == Some("true") {
        return Origin::GitHub {
            repository: var("GITHUB_REPOSITORY").unwrap_or_default(),
            workflow: var("GITHUB_WORKFLOW").unwrap_or_default(),
            run_id: var("GITHUB_RUN_ID")
                .and_then(|id| id.parse().ok())
                .unwrap_or_default(),
            attempt: var("GITHUB_RUN_ATTEMPT")
                .and_then(|id| id.parse().ok())
                .unwrap_or(1),
        };
    }

    let host = rustix::system::uname()
        .nodename()
        .to_string_lossy()
        .split('.')
        .next()
        .unwrap_or_default()
        .to_owned();

    Origin::User {
        user: var("USER")
            .or_else(|| var("LOGNAME"))
            .unwrap_or_else(|| "unknown".into()),
        host,
    }
}
