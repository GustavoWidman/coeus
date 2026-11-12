//! the lines after the checklist settles: the verdict, then what is left for
//! a person, from the evidence only:
//!
//! - merge the pull request
//! - `coeus reboot <host>` when the reboot was declined
//! - on a Mac whose coeusd lacks Full Disk Access, `coeus doctor --fix` on
//!   that Mac
//! - the contract's notes
//! - for a failure, what failed, its last lines, and `--retry` or the file
//!   to fix; for a block, what the person must do, and that running again
//!   picks up from there

use std::path::Path;

use colored::Colorize;

use super::{describe, say};
use crate::{
    bootstrap::{
        Blocker, Evidence, Finale, Step,
        branch::{Branch, PullState},
        contract::RepoPath,
        fleet::Reboot,
        state::StateDir,
    },
    common::model::{FdaStatus, JobState, JobSummary, Outcome},
};

/// the most files a block lists before it counts the rest
const FILES: usize = 5;
/// a failed job's last lines worth showing
const TAIL: usize = 8;

pub fn lines(state: &StateDir, finale: &Finale) -> Vec<String> {
    let host = state.host().as_str();
    let again = format!("`coeus bootstrap {host}`");

    match finale {
        Finale::Done(evidence) => done(evidence),
        Finale::Detached { at } => vec![
            say::aside(detached(host, *at)),
            indent(format!("running {again} again picks up from there")),
        ],
        Finale::Unreached { at, error } => {
            let what = match at {
                Step::Reach => format!("could not reach {host}"),
                at => format!("lost {host} during {at}"),
            };
            vec![
                say::error(format!("{what}: {error:#}")),
                indent(format!(
                    "once it is up and ssh or the tailnet reach it, {again} picks up from there"
                )),
            ]
        }
        Finale::Failed { at, error } => vec![
            say::error(format!("{at} failed: {error:#}")),
            indent(format!("running {again} again tries it again")),
        ],
        Finale::Blocked { at, blocker } => {
            let mut lines = blocked(state, *at, blocker);
            lines.push(indent(format!("then {again} picks up from there")));
            lines
        }
    }
}

fn done(evidence: &Evidence) -> Vec<String> {
    let host = evidence.host.as_str();
    let pull = &evidence.pull_request;

    let mut lines = vec![match pull.state {
        PullState::Merged => say::success(format!("{host} is in the fleet")),
        PullState::Open => say::success(format!("{host} is in the fleet, waiting for review")),
        PullState::Closed => say::warn(format!(
            "{host} runs its fleet configuration, but #{} was closed",
            pull.number
        )),
    }];

    match pull.state {
        PullState::Open => lines.push(indent(format!(
            "merge {}: every other machine learns about {host} on the next rollout",
            pull.url.bold()
        ))),
        PullState::Closed => lines.push(indent(format!(
            "`coeus bootstrap {host}` opens a new pull request, or no other machine learns about {host}"
        ))),
        PullState::Merged => {}
    }

    if evidence.reboot == Reboot::Declined {
        lines.push(indent(format!(
            "it runs a kernel it did not boot: `coeus reboot {host}` when it suits"
        )));
    }
    if evidence.full_disk_access == FdaStatus::Missing {
        lines.push(indent(format!(
            "on {host} itself, run `coeus doctor --fix`: nix-darwin needs Full Disk Access to create or delete users"
        )));
    }
    lines.extend(evidence.notes.iter().map(|note| indent(note.clone())));
    lines
}

fn detached(host: &str, at: Step) -> String {
    match at {
        Step::Switch => format!("detached during the switch: {host} goes on switching alone"),
        Step::Reboot => format!("detached while {host} reboots"),
        at => format!("stopped during {at}; nothing runs on {host}"),
    }
}

fn blocked(state: &StateDir, at: Step, blocker: &Blocker) -> Vec<String> {
    let host = state.host().as_str();
    let branch = Branch::name(state.host());

    match blocker {
        Blocker::NotAdmin { files } => {
            let mut lines = vec![say::warn(format!(
                "this machine cannot open {}, so it is not an admin of the fleet",
                count(files.len(), "secret")
            ))];
            lines.extend(listed(files));
            lines.push(indent(
                "run this from an admin, or have one rekey them first".to_owned(),
            ));
            lines
        }
        Blocker::Unreadable { files } => {
            let mut lines = vec![say::warn(format!(
                "coeus cannot rekey {} without losing a recipient it does not know",
                count(files.len(), "secret")
            ))];
            lines.extend(
                files
                    .iter()
                    .take(FILES)
                    .map(|(file, why)| indent(format!("{file}: {why}").dimmed())),
            );
            if files.len() > FILES {
                lines.push(indent(
                    format!("… and {} more", files.len() - FILES).dimmed(),
                ));
            }
            lines.push(indent(format!(
                "rekey them by hand on {branch}, or have the contract's recipients leave them out"
            )));
            lines
        }
        Blocker::Enrolled { files } => {
            let mut lines = vec![say::warn(format!(
                "{host} is enrolled on the default branch already"
            ))];
            lines.extend(listed(files));
            lines.push(indent(format!(
                "`coeus deploy {host}` updates a machine that is in; to enroll it again, remove those first"
            )));
            lines
        }
        Blocker::Uncommitted { clone, files } => {
            let mut lines = vec![say::warn(format!(
                "{} has changes nobody committed",
                shown(clone)
            ))];
            lines.extend(files.iter().take(FILES).map(|file| indent(file.dimmed())));
            lines.push(indent(
                "commit them on the branch or discard them".to_owned(),
            ));
            lines
        }
        Blocker::Conflict { clone, base } => vec![
            say::warn(format!(
                "{branch} no longer applies onto {base}: {base} changed what it adds"
            )),
            indent(format!(
                "the clone in {} is as it was; rebase it onto origin/{base} there",
                shown(clone)
            )),
        ],
        Blocker::SwitchFailed { job } => {
            let failure = outcome(job).and_then(Outcome::failure);
            let what = match outcome(job) {
                Some(Outcome::RolledBack(_)) => "rolled back",
                Some(Outcome::Interrupted) => "was interrupted",
                Some(Outcome::Cancelled) => "was cancelled",
                _ => "failed",
            };
            let why = failure
                .map(|failure| format!(" {}", describe::failure(failure)))
                .unwrap_or_default();

            let mut lines = vec![say::error(format!(
                "{host}'s first switch, job {}, {what}{why}",
                job.id
            ))];
            if let Some(failure) = failure {
                let tail = &failure.tail[failure.tail.len().saturating_sub(TAIL)..];
                lines.extend(tail.iter().map(|line| indent(format!("│ {line}").dimmed())));
            }
            if matches!(outcome(job), Some(Outcome::RolledBack(_))) {
                lines.push(indent(format!("{host} runs the system it ran before")));
            }
            lines.push(indent(format!(
                "commit a fix on {branch} in {}, or `coeus bootstrap {host} --retry` to try this commit again",
                shown(&state.clone_dir())
            )));
            lines
        }
        Blocker::AdoptMissing { version } => vec![
            say::warn(format!(
                "the coeus {host}'s configuration pins ({version}) has no `coeusd adopt`"
            )),
            indent("update the flake input that provides coeus on the branch".to_owned()),
        ],
        Blocker::RootRefused { user } => vec![
            say::warn(format!(
                "root on {host} refuses coeus's session key, and its nix takes nothing unsigned from {user}"
            )),
            indent(format!(
                "let root log in with a key (PermitRootLogin prohibit-password), make {user} a trusted nix user, or let it sudo without a password"
            )),
        ],
        Blocker::OffTailnet => vec![
            say::warn(format!("{host} is up but not on the tailnet")),
            indent(format!("sign in to Tailscale on {host}")),
        ],
        Blocker::Forge(why) => vec![
            say::warn(format!("gh could not do it: {why}")),
            indent("`gh auth status` says whether it is signed in".to_owned()),
        ],
        Blocker::Unanswered { id } => vec![
            say::warn(format!(
                "the repository asks `{id}`, and there is no terminal to ask it on"
            )),
            indent(format!(
                "answer it with `--answer {id}=<value>`, or run this in a terminal"
            )),
        ],
        Blocker::NeedsTerminal { secret } => vec![
            say::warn(format!("{secret} is typed by a person, at {at}")),
            indent("run this in a terminal".to_owned()),
        ],
    }
}

fn outcome(job: &JobSummary) -> Option<&Outcome> {
    match &job.state {
        JobState::Finished(outcome) => Some(outcome),
        _ => None,
    }
}

fn listed(files: &[RepoPath]) -> Vec<String> {
    let mut lines: Vec<String> = files
        .iter()
        .take(FILES)
        .map(|file| indent(file.as_str().dimmed()))
        .collect();
    if files.len() > FILES {
        lines.push(indent(
            format!("… and {} more", files.len() - FILES).dimmed(),
        ));
    }
    lines
}

fn count(count: usize, noun: &str) -> String {
    match count {
        1 => format!("1 {noun}"),
        count => format!("{count} {noun}s"),
    }
}

/// under `[✓] ` of the line before
fn indent(text: impl std::fmt::Display) -> String {
    format!("    {text}")
}

/// a path under the home directory as `~/…`
fn shown(path: &Path) -> String {
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    match home
        .as_deref()
        .and_then(|home| path.strip_prefix(home).ok())
    {
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}
