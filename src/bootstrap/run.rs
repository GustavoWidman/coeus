//! one pass over the steps. each one observes, acts on the difference,
//! observes again and requires done: an act that does not leave its step
//! observably done is a bug, reported by the step's name.

use super::{
    Bootstrap, Finale,
    branch::{Branch, Enrolled, PullRequest},
    remote::Target,
    switch::{Release, Switched},
};
use crate::cli::Console;

pub async fn run(bootstrap: &mut Bootstrap, console: &Console) -> eyre::Result<Finale> {
    // TODO:
    // let _lock = bootstrap.state.lock()?;
    // let target = step(Reach, remote::reach(bootstrap, console))
    //     // coeusd and ssh tried at once; ssh opened lazily when coeusd answers
    // if the machine is a Mac: step(Nix, remote::converge_nix(stock, console))
    // let branch = Branch::open(&state, saved.repo, host)
    // let enrolled = step(Enroll, enroll(&branch, &target, bootstrap, console))
    // let release = step(Rekey, rekey(&branch, &enrolled, bootstrap, console))
    // let pull_request = step(Push, push(&branch, &enrolled, console))
    // let switched = step(Switch, switch::converge(&release, &target, bootstrap, console))
    // let answered = step(Answer, fleet::answer(&target, &switched, console))
    // let reboot = step(Reboot, fleet::reboot(&answered, console))
    // step(Review, review(&branch, pull_request, evidence, console))
    // step(Cleanup, SessionKey::existing(state)?.remove(stock))
    //
    // `step` maps a step's error to Finale::Failed, a Blocker to
    // Finale::Blocked and the console's cancellation to Finale::Detached,
    // naming the step
    todo!()
}

/// the contract's questions, the machine's files and its own secrets,
/// committed once. found done by the enroll trailer.
async fn enroll(
    branch: &Branch,
    target: &Target,
    bootstrap: &mut Bootstrap,
    console: &Console,
) -> eyre::Result<Enrolled> {
    // TODO:
    // if let Some(enrolled) = branch.enrolled()? { return Ok(enrolled) }
    // let contract = Contract::open(clone, DEFAULT_PATH)
    // loop: the first question not answered (saved or --answer) is asked,
    //       saved at once (an answer is never asked twice), and the
    //       questions evaluated again
    // let enrollment = contract.enroll(&input)
    // branch.on_base(declared paths) non-empty → Blocker::Enrolled
    // branch.write(files)
    // let rules = contract.recipients(secret paths)
    // each absent secret: ask what it needs, generate, secrets::create
    // branch.commit_enroll(messages.enroll, declared paths)
    todo!()
}

/// the branch on the default branch's head, every secret on its recipients,
/// and the release the machine should run
async fn rekey(
    branch: &Branch,
    enrolled: &Enrolled,
    bootstrap: &Bootstrap,
    console: &Console,
) -> eyre::Result<Release> {
    // TODO:
    // if branch.behind()? > 0 { branch.rebase_dropping_rekey()? }   // Blocker::Conflict
    // branch.uncommitted() non-empty → Blocker::Uncommitted
    // let plan = secrets::plan(clone, contract.recipients(branch.secrets()))
    // if a file is stale: AdminIdentity::public(identity), plan.unopenable_by
    //     → Blocker::NotAdmin before any sudo; console.hand_over(unlock);
    //     plan.apply; branch.commit_rekey(messages.rekey)
    // plan again: nothing stale, or bail naming the files
    todo!()
}

/// the branch on origin and a draft pull request for it
async fn push(
    branch: &Branch,
    enrolled: &Enrolled,
    console: &Console,
) -> eyre::Result<PullRequest> {
    todo!()
}

/// coeus's section of the pull request rewritten from the evidence, and the
/// draft marked ready
async fn review(
    branch: &Branch,
    pull_request: PullRequest,
    switched: &Switched,
    console: &Console,
) -> eyre::Result<PullRequest> {
    todo!()
}
