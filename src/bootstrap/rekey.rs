//! the rekey step: the branch on its base's head, every secret encrypted to
//! exactly its recipients, and the release the machine should run. found
//! done when the branch is not behind and every header names its
//! recipients.

use std::path::Path;

use eyre::{Result, WrapErr, bail};

use super::{
    Blocker, Bootstrap, Step,
    branch::{Branch, Rebased},
    contract::{RepoPath, Rules},
    enroll::{Enrolled, count},
    run::Shown,
    secrets::{self, AdminIdentity, Verdict},
    switch::Release,
};
use crate::{
    cli::{Console, local_hostname},
    client::Snapshot,
    ui::checklist::{Live, Mark, Row},
};

pub struct Rekeyed {
    pub release: Release,
    /// the secrets the machine opens besides its own
    pub opens: Vec<RepoPath>,
}

pub async fn rekey(
    enrolled: &Enrolled,
    bootstrap: &Bootstrap,
    console: &Console,
) -> Result<Shown<Rekeyed>> {
    let branch = &enrolled.branch;
    let clone = branch.clone_dir();
    let subject = &enrolled.enrollment.messages.rekey;

    // secrets a stopped run rewrote but did not commit are its rekey
    let finished = settle(branch).await?;
    if !finished.is_empty() {
        branch.commit_rekey(subject, &finished).await?;
    }

    let mut rebased = false;
    if branch.behind().await? > 0 {
        console.mark(
            Step::Rekey,
            Mark::Active,
            format!("moving the branch onto {}", branch.base()),
        );
        if branch.rebase_dropping_rekey().await? == Rebased::Conflict {
            return Err(Blocker::Conflict {
                clone: clone.to_owned(),
                base: branch.base().to_owned(),
            }
            .into());
        }
        rebased = true;
    }

    let rules = enrolled
        .contract
        .recipients(&branch.secrets().await?)
        .await?;
    let rekeyed = seal(enrolled, &rules, bootstrap, console).await?;

    let again = secrets::plan(clone, &rules)?;
    let stale: Vec<&str> = again.stale().map(|file| file.path.as_str()).collect();
    if !stale.is_empty() {
        bail!(
            "rekeyed, yet {} still name other recipients",
            stale.join(", ")
        );
    }
    let changed = changed_secrets(branch).await?;
    let commit = match changed.is_empty() {
        true => None,
        false => Some(branch.commit_rekey(subject, &changed).await?),
    };
    let left = branch.uncommitted().await?;
    if !left.is_empty() {
        bail!("the clone has changes after the rekey: {}", left.join(", "));
    }

    let release = release(enrolled, bootstrap).await?;
    let own = &enrolled.enrollment.secrets;
    let opens = rules
        .iter()
        .filter(|(path, keys)| {
            !own.contains_key(*path)
                && keys
                    .as_ref()
                    .is_some_and(|keys| keys.contains(&enrolled.host_key))
        })
        .map(|(path, _)| path.clone())
        .collect();

    let kept = finished.is_empty() && !rebased && rekeyed == 0;
    let mut detail = Vec::new();
    if rebased {
        detail.push(format!("moved onto {}", branch.base()));
    }
    detail.push(match (rekeyed + finished.len(), commit) {
        (0, _) => "every secret current".to_owned(),
        (count_rekeyed, Some(commit)) => format!(
            "{} rekeyed for {} ({})",
            count(count_rekeyed, "secret"),
            bootstrap.host,
            commit.short()
        ),
        (count_rekeyed, None) => format!("{} rekeyed", count(count_rekeyed, "secret")),
    });
    Ok(Shown {
        value: Rekeyed { release, opens },
        mark: if kept { Mark::Kept } else { Mark::Done },
        detail: detail.join(" · "),
    })
}

/// what a stopped run left in the clone: its temporary files go; the
/// secrets it rewrote come back to be committed; anything else is the
/// person's, and blocks
async fn settle(branch: &Branch) -> Result<Vec<RepoPath>> {
    let tracked = branch.secrets().await?;
    let mut finished = Vec::new();
    let mut theirs = Vec::new();
    for path in branch.uncommitted().await? {
        let file = branch.clone_dir().join(&path);
        if leftover(&path) {
            std::fs::remove_file(&file).ok();
            continue;
        }
        match tracked.iter().find(|secret| secret.as_str() == path) {
            Some(secret) if file.is_file() => finished.push(secret.clone()),
            _ => theirs.push(path),
        }
    }
    if !theirs.is_empty() {
        return Err(Blocker::Uncommitted {
            clone: branch.clone_dir().to_owned(),
            files: theirs,
        }
        .into());
    }
    Ok(finished)
}

/// the temporary sibling a secret is written through, `.<name>.age.<random>.tmp`
fn leftover(path: &str) -> bool {
    let name = Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    name.starts_with('.') && name.ends_with(".tmp") && name.contains(".age.")
}

/// the stale secrets re-encrypted with the admin's identity; how many
async fn seal(
    enrolled: &Enrolled,
    rules: &Rules,
    bootstrap: &Bootstrap,
    console: &Console,
) -> Result<usize> {
    let clone = enrolled.branch.clone_dir();
    let plan = secrets::plan(clone, rules)?;
    let unreadable: Vec<String> = plan
        .files
        .iter()
        .filter_map(|file| match &file.verdict {
            Verdict::Unreadable(why) => Some(format!("{} ({why})", file.path)),
            _ => None,
        })
        .collect();
    if !unreadable.is_empty() {
        bail!(
            "coeus cannot read who these are encrypted to: {}",
            unreadable.join(", ")
        );
    }
    let stale = plan.stale().count();
    if stale == 0 {
        return Ok(0);
    }

    let identity = bootstrap
        .options
        .identity
        .clone()
        .unwrap_or_else(|| enrolled.contract.identity().to_owned());
    plan.require_admin(&AdminIdentity::public(&identity)?)?;
    console.mark(
        Step::Rekey,
        Mark::Active,
        format!(
            "{} must also open on {}",
            count(stale, "secret"),
            bootstrap.host
        ),
    );
    let identity = match std::fs::File::open(&identity) {
        Ok(_) => AdminIdentity::unlock(&identity).await?,
        Err(_) => {
            let why = format!(
                "sudo reads {} to rekey {}",
                identity.display(),
                count(stale, "secret")
            );
            console
                .hand_over(&why, AdminIdentity::unlock(&identity))
                .await?
        }
    };
    let rewritten = plan.apply(&identity, |done, total, path| {
        console.update(Row {
            step: Step::Rekey,
            mark: Mark::Active,
            detail: format!("rekeying for {}", bootstrap.host),
            live: vec![Live::Count {
                done,
                total,
                current: path.to_string(),
            }],
        });
    })?;
    Ok(rewritten.len())
}

/// tracked secrets that differ from the head
async fn changed_secrets(branch: &Branch) -> Result<Vec<RepoPath>> {
    let tracked = branch.secrets().await?;
    let uncommitted = branch.uncommitted().await?;
    Ok(tracked
        .into_iter()
        .filter(|secret| uncommitted.iter().any(|path| path == secret.as_str()))
        .collect())
}

/// the clone's committed head as the snapshot the machine switches to
async fn release(enrolled: &Enrolled, bootstrap: &Bootstrap) -> Result<Release> {
    let branch = &enrolled.branch;
    let head = branch.head().await?;
    let snapshot = Snapshot::capture(branch.clone_dir())
        .await
        .wrap_err("could not take a snapshot of the clone")?;
    Ok(Release {
        head,
        nar_hash: snapshot.nar_hash,
        label: format!(
            "{}:{}@{}",
            local_hostname(),
            Branch::name(&bootstrap.host),
            head.short()
        ),
        install_bootloader: enrolled.enrollment.first_switch.install_bootloader,
    })
}
