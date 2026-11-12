//! one pass over the steps. each one observes, acts on the difference,
//! observes again and requires done: an act that does not leave its step
//! observably done is an error, reported by the step's name.

use std::future::Future;

use eyre::Result;

use super::{
    Blocker, Bootstrap, Evidence, Finale, Step,
    branch::{GhError, body},
    enroll::{self, Enrolled},
    fleet::Reboot,
    publish,
    rekey::{self, Rekeyed},
    state::{Ended, LastRun},
    switch::Switched,
    world::{Answer, Forge, Machine},
};
use crate::{
    cli::{Console, Unanswerable, local_hostname},
    common::model::{Platform, Timestamp},
    ui::{
        checklist::{Mark, Row},
        format,
    },
};

/// a step's result, and what its row says about it
pub struct Shown<T> {
    pub value: T,
    /// [Mark::Kept] when it was found done, [Mark::Done] when this run did
    /// it, [Mark::Skipped] when it does not apply
    pub mark: Mark,
    pub detail: String,
}

/// the pass, holding the machine's lock, with how it ended kept as the last
/// run for the next one to show
pub(crate) async fn run<M: Machine, F: Forge>(
    bootstrap: &mut Bootstrap,
    machine: &M,
    forge: &F,
    console: &Console,
) -> Result<Finale> {
    let _lock = bootstrap.state.lock().await?;
    let started_at = Timestamp::now();
    bootstrap.saved.last_run = Some(LastRun {
        started_at,
        ended: None,
    });
    bootstrap.state.save(&bootstrap.saved)?;

    let finale = match pass(bootstrap, machine, forge, console, started_at).await {
        Ok(finale) | Err(finale) => finale,
    };

    bootstrap.saved.last_run = Some(LastRun {
        started_at,
        ended: Some(Ended {
            at: Timestamp::now(),
            how: how(&finale),
        }),
    });
    bootstrap.state.save(&bootstrap.saved)?;
    Ok(finale)
}

async fn pass<M: Machine, F: Forge>(
    bootstrap: &mut Bootstrap,
    machine: &M,
    forge: &F,
    console: &Console,
    started_at: Timestamp,
) -> Result<Finale, Finale> {
    let platform = step(console, Step::Reach, async {
        let reached = machine.reach(bootstrap, console).await?;
        let mark = if reached.kept { Mark::Kept } else { Mark::Done };
        Ok(shown(reached.platform, mark, reached.detail))
    })
    .await?;

    let mut nix_version = None;
    if platform == Platform::Darwin {
        nix_version = step(console, Step::Nix, async {
            let ready = machine.nix(bootstrap, console).await?;
            for file in &ready.moved {
                console.note(format!("moved {file}, so nix-darwin can write its own"));
            }
            let (mark, detail) = match (&ready.version, ready.installed || !ready.moved.is_empty())
            {
                (None, _) => (Mark::Kept, "nix-darwin's coeusd answers".to_owned()),
                (Some(version), false) => (Mark::Kept, format!("Determinate Nix {version}")),
                (Some(version), true) => (Mark::Done, format!("Determinate Nix {version}")),
            };
            Ok(shown(ready.version, mark, detail))
        })
        .await?;
    }

    let enrolled = step(
        console,
        Step::Enroll,
        enroll::enroll(bootstrap, machine, nix_version, console),
    )
    .await?;
    let rekeyed = step(
        console,
        Step::Rekey,
        rekey::rekey(&enrolled, bootstrap, console),
    )
    .await?;

    let mut evidence = evidence(&enrolled, &rekeyed);
    let pull = step(
        console,
        Step::Push,
        publish::push(&enrolled, &evidence, forge, console),
    )
    .await?;

    let release = &rekeyed.release;
    let switched = step(console, Step::Switch, async {
        let switched = machine.switch(release, bootstrap, console).await?;
        let earlier = switched
            .job
            .finished_at
            .is_some_and(|finished| finished < started_at);
        let mark = if earlier { Mark::Kept } else { Mark::Done };
        let detail = switch_detail(&switched);
        Ok(shown(switched, mark, detail))
    })
    .await?;
    let kept = |earlier: bool| if earlier { Mark::Kept } else { Mark::Done };
    let switched_earlier = switched
        .job
        .finished_at
        .is_some_and(|finished| finished < started_at);
    evidence.switch = Some(switch_detail(&switched));

    let answer = step(console, Step::Answer, async {
        let answer = machine.answer(&switched, console).await?;
        let detail = format!(
            "coeusd {} on {} ({})",
            answer.version, bootstrap.host, answer.address
        );
        Ok(shown(answer, kept(switched_earlier), detail))
    })
    .await?;
    evidence.coeusd = Some(format!("{} at {}", answer.version, answer.address));

    let reboot = step(console, Step::Reboot, reboot(machine, &answer, console)).await?;
    evidence.boot = Some(boot(&reboot).to_owned());

    let pull = step(
        console,
        Step::Review,
        publish::review(&enrolled, pull, &evidence, forge, console),
    )
    .await?;

    step(console, Step::Cleanup, async {
        let removed = machine.cleanup(bootstrap, console).await?;
        Ok(match removed {
            true => shown(
                (),
                Mark::Done,
                "the session key is out of root's authorized_keys",
            ),
            false => shown((), Mark::Kept, "no session key left on the machine"),
        })
    })
    .await?;

    Ok(Finale::Done(Box::new(Evidence {
        host: bootstrap.host.clone(),
        pull_request: pull,
        section: evidence,
        notes: enrolled.enrollment.notes.clone(),
        reboot,
        full_disk_access: answer.full_disk_access,
    })))
}

/// a step's row while it works and once it is done, or the finale it ends
/// the run with
async fn step<T>(
    console: &Console,
    step: Step,
    work: impl Future<Output = Result<Shown<T>>>,
) -> Result<T, Finale> {
    if console.cancelled.is_cancelled() {
        return Err(Finale::Detached { at: step });
    }
    console.mark(step, Mark::Active, looking(step));
    match work.await {
        Ok(done) => {
            console.update(Row::new(step, done.mark, done.detail));
            Ok(done.value)
        }
        Err(error) => Err(ended(console, step, error)),
    }
}

/// what a step does first: look
const fn looking(step: Step) -> &'static str {
    match step {
        Step::Reach => "asking its coeusd, else ssh",
        Step::Nix => "looking for nix",
        Step::Enroll => "reading the branch",
        Step::Rekey => "reading who each secret opens for",
        Step::Push => "comparing the branch with origin",
        Step::Switch => "looking for its switch",
        Step::Answer => "asking its coeusd over the tailnet",
        Step::Reboot => "asking whether it booted what it runs",
        Step::Review => "reading the pull request",
        Step::Cleanup => "looking for the session key",
    }
}

fn shown<T>(value: T, mark: Mark, detail: impl Into<String>) -> Shown<T> {
    Shown {
        value,
        mark,
        detail: detail.into(),
    }
}

/// what a step's error means for the run: ctrl-c detaches, a blocker waits
/// for a person, the first step not reaching the machine is unreached, and
/// anything else failed
fn ended(console: &Console, at: Step, error: eyre::Report) -> Finale {
    if console.cancelled.is_cancelled() {
        console.mark(at, Mark::Pending, "");
        return Finale::Detached { at };
    }
    if let Some(blocker) = blocker(&error) {
        console.mark(at, Mark::Blocked, blocker.to_string());
        return Finale::Blocked { at, blocker };
    }
    let first = format!("{error}");
    console.mark(at, Mark::Failed, first.lines().next().unwrap_or_default());
    match at {
        Step::Reach => Finale::Unreached { at, error },
        at => Finale::Failed { at, error },
    }
}

/// a blocker anywhere in the error's chain; gh failing and a question with
/// nobody to answer it are blockers too
fn blocker(error: &eyre::Report) -> Option<Blocker> {
    error.chain().find_map(|cause| {
        if let Some(blocker) = cause.downcast_ref::<Blocker>() {
            return Some(blocker.clone());
        }
        if let Some(gh) = cause.downcast_ref::<GhError>() {
            return Some(Blocker::Forge(gh.to_string()));
        }
        cause
            .downcast_ref::<Unanswerable>()
            .and_then(|unanswerable| unanswerable.id.clone())
            .map(|id| Blocker::Unanswered { id })
    })
}

async fn reboot<M: Machine>(
    machine: &M,
    answer: &Answer,
    console: &Console,
) -> Result<Shown<Reboot>> {
    if !answer.reboot_required {
        return Ok(shown(Reboot::NotNeeded, Mark::Skipped, ""));
    }
    let reboot = machine.reboot(answer, console).await?;
    let detail = match &reboot {
        Reboot::Rebooted { took } => format!("back in {}", format::duration(*took)),
        Reboot::Declined => "later: it runs a kernel it did not boot".to_owned(),
        Reboot::NotNeeded => String::new(),
    };
    Ok(shown(reboot, Mark::Done, detail))
}

/// the pull request's section as far as the run got before the switch
fn evidence(enrolled: &Enrolled, rekeyed: &Rekeyed) -> body::Evidence {
    let enrollment = &enrolled.enrollment;
    body::Evidence {
        host: enrolled.host.to_string(),
        base: enrolled.branch.base().to_owned(),
        machine: enrolled.facts.describe(),
        host_key: enrolled.host_key.to_string(),
        fingerprint: enrolled.host_key.fingerprint(),
        enrolled_by: local_hostname(),
        files: enrollment.files.keys().cloned().collect(),
        created: enrollment.secrets.keys().cloned().collect(),
        rekeyed: rekeyed.opens.clone(),
        switch: None,
        coeusd: None,
        boot: None,
    }
}

/// `job #1 · nixos-system-nova-26.05 · 7m12s`; the same on every run, so
/// the pull request's section does not change for nothing
fn switch_detail(switched: &Switched) -> String {
    let job = &switched.job;
    let mut parts = vec![format!("job {}", job.id)];
    if let Some(toplevel) = &job.toplevel {
        parts.push(toplevel.name().to_owned());
    }
    if let Some(took) = job.duration() {
        parts.push(format::duration(took));
    }
    parts.join(" · ")
}

fn boot(reboot: &Reboot) -> &'static str {
    match reboot {
        Reboot::NotNeeded | Reboot::Rebooted { .. } => "boots the system it runs",
        Reboot::Declined => "runs a kernel it did not boot; a reboot is due",
    }
}

/// how the run ended, for the next run and the list of bootstraps
fn how(finale: &Finale) -> String {
    match finale {
        Finale::Done(_) => "in the fleet".to_owned(),
        Finale::Detached { at } => format!("detached during {at}"),
        Finale::Unreached { at, .. } => format!("unreached during {at}"),
        Finale::Failed { at, .. } => format!("{at} failed"),
        Finale::Blocked { at, blocker } => format!("blocked at {at}: {blocker}"),
    }
}
