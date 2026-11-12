//! the first switch's next move, decided from the snapshot's jobs on the
//! machine and from what it runs. NixOS may refuse to switch live into the
//! fleet's system in a pre-switch check, asking for a boot and a reboot
//! instead: that is done without asking, as the snapshot's next try. a boot
//! that succeeded is done once the machine runs its system, else the reboot
//! is still owed; that is read from the machine, never remembered.

use eyre::{Result, bail};

use super::Ending;
use crate::{
    bootstrap::{Blocker, remote::Running},
    cli::Console,
    common::model::{
        Action, Attempt, Hostname, JobState, JobSummary, OrderId, Outcome, Phase, Platform,
        StorePath,
    },
};

/// the stock machine as the first switch goes through it
pub(crate) trait Stocked {
    /// where the snapshot's switch stands
    async fn status(&mut self) -> Result<Attempt>;

    /// what the machine runs now
    async fn running(&mut self) -> Result<Running>;

    /// `order` admitted as `action` unless it was already, its job followed
    async fn adopt(&mut self, order: OrderId, action: Action) -> Result<Ending>;

    /// the machine told to reboot into the system its boot job made default
    async fn reboot(&mut self) -> Result<()>;
}

/// what the first switch does next
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Next {
    /// admit this try as `action` and follow it
    Order { order: OrderId, action: Action },
    /// NixOS refused to switch live: this try boots into the system instead
    Boot { order: OrderId, refusal: Refusal },
    /// follow the try that runs
    Follow(JobSummary),
    /// a boot that succeeded: done once the machine runs its system
    Booted(JobSummary),
    /// the machine runs the snapshot
    Done(JobSummary),
    /// the last try failed; `--retry` orders another
    Failed(JobSummary),
    /// every try failed
    Exhausted,
}

impl Next {
    pub fn of(attempt: Attempt, platform: Platform, retry: bool) -> Self {
        match attempt {
            Attempt::Fresh(order) => Self::Order {
                order,
                action: Action::Switch,
            },
            Attempt::Running(job) => Self::Follow(job),
            Attempt::Succeeded(job) if job.action == Action::Boot => Self::Booted(job),
            Attempt::Succeeded(job) => Self::Done(job),
            Attempt::Failed { last, next } => match (Refusal::of(&last, platform), next) {
                (Some(refusal), Some(order)) => Self::Boot { order, refusal },
                (Some(_), None) => Self::Exhausted,
                (None, Some(order)) if retry => Self::Order {
                    order,
                    action: again(&last),
                },
                (None, None) if retry => Self::Exhausted,
                (None, _) => Self::Failed(last),
            },
        }
    }
}

/// a retry does what the try before it did: a boot that failed is booted
/// again, since the switch it stands for was refused
fn again(last: &JobSummary) -> Action {
    match last.action {
        Action::Boot => Action::Boot,
        _ => Action::Switch,
    }
}

/// NixOS refusing a live switch in a pre-switch check, in its own words
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refusal {
    /// the check's last lines, up to the first naming a pre-switch check
    pub lines: Vec<String>,
}

impl Refusal {
    /// what `switch-to-configuration` prints when a check refuses, either
    /// `Pre-switch check '<name>' failed` or `Pre-switch checks failed`
    const SAID: &str = "Pre-switch check";
    /// how many of its lines the note quotes
    const QUOTED: usize = 3;

    /// the refusal a switch on NixOS ended with, if it ended with one
    pub fn of(job: &JobSummary, platform: Platform) -> Option<Self> {
        if platform != Platform::NixOs || job.action != Action::Switch {
            return None;
        }
        let failure = match &job.state {
            JobState::Finished(Outcome::RolledBack(failure) | Outcome::Failed(failure)) => failure,
            _ => return None,
        };
        if failure.phase != Phase::Activating {
            return None;
        }

        let said = failure
            .tail
            .iter()
            .position(|line| line.contains(Self::SAID))?;
        let lines: Vec<String> = failure.tail[..=said]
            .iter()
            .map(|line| line.trim())
            .filter(|line| !line.is_empty())
            .map(str::to_owned)
            .collect();
        let quoted = lines.len().saturating_sub(Self::QUOTED);
        Some(Self {
            lines: lines[quoted..].to_vec(),
        })
    }

    pub fn note(&self) -> String {
        format!(
            "NixOS asks for a reboot into this system instead of a live switch: {}",
            self.lines.join("; ")
        )
    }
}

/// whether a try followed to its end leaves the first switch more to do: a
/// refused switch is booted instead, a boot is rebooted into
pub fn goes_on(job: &JobSummary, platform: Platform) -> bool {
    let succeeded = matches!(
        job.state,
        JobState::Finished(Outcome::Succeeded | Outcome::UpToDate)
    );
    Refusal::of(job, platform).is_some() || (job.action == Action::Boot && succeeded)
}

/// whether a boot job's system still waits for a reboot: the machine runs
/// something else
pub fn owes_reboot(job: &JobSummary, running: Option<&StorePath>) -> bool {
    job.toplevel.is_some() && job.toplevel.as_ref() != running
}

/// where a machine told to reboot into `job`'s system is, as ssh finds it
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Back {
    /// it has not gone down yet
    NotYet,
    /// it runs the boot job's system
    Home,
    /// it booted again, into something else
    Elsewhere(Option<StorePath>),
}

impl Back {
    pub fn of(job: &JobSummary, before: &Running, now: &Running) -> Self {
        if now.system.is_some() && now.system == job.toplevel {
            return Self::Home;
        }
        match now.boot.is_some() && now.boot != before.boot {
            true => Self::Elsewhere(now.system.clone()),
            false => Self::NotYet,
        }
    }
}

/// decides, acts and decides again until the machine runs the snapshot, a
/// try fails, or the machine was told to reboot into it
pub(crate) async fn converge<S: Stocked>(
    stocked: &mut S,
    host: &Hostname,
    platform: Platform,
    retry: bool,
    console: &Console,
) -> Result<Ending> {
    loop {
        let ending = match Next::of(stocked.status().await?, platform, retry) {
            Next::Done(job) => return Ok(Ending::Finished(job)),
            Next::Failed(job) => return Err(Blocker::SwitchFailed { job: Box::new(job) }.into()),
            Next::Exhausted => {
                bail!(
                    "every try of this head failed on {host}; commit a fix in the clone and run again"
                );
            }
            Next::Order { order, action } => stocked.adopt(order, action).await?,
            Next::Boot { order, refusal } => {
                console.note(refusal.note());
                stocked.adopt(order, Action::Boot).await?
            }
            Next::Follow(job) => stocked.adopt(job.order, job.action).await?,
            Next::Booted(job) => {
                let before = stocked.running().await?;
                if !owes_reboot(&job, before.system.as_ref()) {
                    return Ok(Ending::Finished(job));
                }
                stocked.reboot().await?;
                return Ok(Ending::Rebooted { job, before });
            }
        };
        match ending {
            Ending::Finished(job) if goes_on(&job, platform) => continue,
            ending => return Ok(ending),
        }
    }
}
