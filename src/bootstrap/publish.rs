//! the push and review steps: the branch on origin with a draft pull request
//! before the switch, and the pull request saying what happened and ready
//! for review after it

use eyre::{Result, eyre};

use super::{
    Step,
    branch::{Branch, PullRequest, PullState, body},
    enroll::Enrolled,
    run::Shown,
    world::Forge,
};
use crate::{cli::Console, common::model::CommitSha, ui::checklist::Mark};

/// the branch on origin and its pull request, opened as a draft when there
/// is none
pub async fn push<F: Forge>(
    enrolled: &Enrolled,
    evidence: &body::Evidence,
    forge: &F,
    console: &Console,
) -> Result<Shown<PullRequest>> {
    let branch = &enrolled.branch;
    let clone = branch.clone_dir();
    let name = Branch::name(&enrolled.host);

    console.mark(Step::Push, Mark::Active, format!("pushing {name}"));
    let pushed = branch.push().await?;

    let opened = match forge.find(clone, &name).await? {
        Some(_) => false,
        None => {
            console.mark(Step::Push, Mark::Active, "opening a draft pull request");
            let messages = &enrolled.enrollment.messages;
            let body = body::splice(&messages.body, &body::section(evidence));
            forge
                .create_draft(clone, &name, branch.base(), &messages.title, &body)
                .await?;
            true
        }
    };

    let pull = forge
        .find(clone, &name)
        .await?
        .ok_or_else(|| eyre!("gh finds no pull request for {name} after opening one"))?;
    let (mark, detail) = match (pushed || opened, pull.draft) {
        (false, _) => (Mark::Kept, format!("#{} up to date", pull.number)),
        (true, true) => (Mark::Done, format!("{name} → draft #{}", pull.number)),
        (true, false) => (Mark::Done, format!("{name} → #{}", pull.number)),
    };
    Ok(Shown {
        value: pull,
        mark,
        detail,
    })
}

/// coeus's section rewritten from the evidence, and the draft marked ready,
/// once the pull request shows the head this run switched the machine to
pub async fn review<F: Forge>(
    enrolled: &Enrolled,
    pull: PullRequest,
    evidence: &body::Evidence,
    forge: &F,
    console: &Console,
) -> Result<Shown<PullRequest>> {
    let clone = enrolled.branch.clone_dir();
    let head = enrolled.branch.head().await?;
    let name = Branch::name(&enrolled.host);
    let section = body::section(evidence);
    let said = |pull: &PullRequest| body::splice(&pull.body, &section) == pull.body;
    let ready = |pull: &PullRequest| !pull.draft || pull.state != PullState::Open;

    // the forge as it is now: the push step read it right after pushing
    let mut pull = forge
        .find(clone, &name)
        .await?
        .filter(|found| found.number == pull.number)
        .ok_or_else(|| eyre!("gh no longer finds #{} for {name}", pull.number))?;
    shows(&pull, head)?;
    let kept = said(&pull) && ready(&pull);

    console.mark(
        Step::Review,
        Mark::Active,
        format!("saying what happened in #{}", pull.number),
    );
    forge.edit_body_section(clone, &mut pull, &section).await?;
    if pull.state == PullState::Open {
        forge.ready(clone, &mut pull).await?;
    }

    let pull = forge
        .find(clone, &name)
        .await?
        .ok_or_else(|| eyre!("gh no longer finds the pull request for {name}"))?;
    shows(&pull, head)?;
    if !said(&pull) || !ready(&pull) {
        return Err(eyre!(
            "#{} does not show coeus's section, or is still a draft",
            pull.number
        ));
    }
    let detail = match pull.state {
        PullState::Open => format!("#{} ready for review", pull.number),
        PullState::Merged => format!("#{} merged", pull.number),
        PullState::Closed => format!("#{} closed", pull.number),
    };
    Ok(Shown {
        value: pull,
        mark: if kept { Mark::Kept } else { Mark::Done },
        detail,
    })
}

/// the pull request is of the commit the machine runs: coeus says what
/// happened only in its own pull request, about its own commit
fn shows(pull: &PullRequest, head: CommitSha) -> Result<()> {
    if pull.head != head {
        return Err(eyre!(
            "#{} is of {}, not of {} which this run pushed; running again looks again",
            pull.number,
            pull.head.short(),
            head.short()
        ));
    }
    Ok(())
}
