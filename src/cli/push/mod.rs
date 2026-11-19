//! one push notification per deploy or rollback the CLI followed to its
//! end, summarizing every machine of the order

use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, SystemTime},
};

use super::deployment::append_step_summary;
use crate::{
    client::Unreach,
    common::model::{Action, Hostname, Order, Outcome, Rejection, Source},
    config::PushSettings,
    push::{Delivery, Notice, Push},
    ui::{
        Board,
        board::{Row, Status},
        describe, format, say,
    },
};

/// how one machine of the order ended
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Machine {
    pub host: Hostname,
    pub end: End,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum End {
    Finished(Outcome),
    Rejected(Rejection),
    /// pending, never failed: it catches up once it is back
    Unreachable(Unreach),
    /// never ordered: the rollout stopped first
    Held,
}

/// every settled machine of the board
pub fn machines(board: &Board) -> Vec<Machine> {
    board.rows.iter().filter_map(Machine::of).collect()
}

impl Machine {
    fn of(row: &Row) -> Option<Self> {
        let end = match row.status() {
            Status::Finished(_, outcome) => End::Finished(outcome.clone()),
            Status::Rejected(rejection) => End::Rejected(rejection.clone()),
            Status::Unreachable(reason) => End::Unreachable(reason.clone()),
            Status::Held => End::Held,
            _ => return None,
        };
        Some(Self {
            host: row.host.clone(),
            end,
        })
    }
}

const TITLE: usize = 64;
const SUBJECT: usize = 100;
const MESSAGE: usize = 80;
const DETAILS: usize = 300;

/// the summary of `order`, ended on `machines` after `took`. `subject` is
/// the deployed commit's subject, when known
pub fn deploy_notice(
    order: &Order,
    machines: &[Machine],
    took: Duration,
    subject: Option<&str>,
) -> Notice {
    let tally = Tally::of(order.action, &order.source, machines);
    let lines = [
        subject.map(|subject| clip(subject, SUBJECT)),
        tally.counts(machines.len(), took),
        tally.details(),
    ];

    Notice {
        title: clip(&title(order, machines, &tally), TITLE),
        body: lines.into_iter().flatten().collect::<Vec<_>>().join("\n"),
        path: format!("/deployments/{}", order.id),
        tag: tag(order, machines),
        urgent: tally.failed(),
    }
}

/// the deploy of `git_ref` no machine could resolve to a commit
pub fn unresolved_notice(git_ref: &str, action: Action) -> Notice {
    Notice {
        title: clip(&format!("Deploy of {git_ref} reached no machine"), TITLE),
        body: format!("no machine could resolve {git_ref}"),
        path: "/".into(),
        tag: format!("deploy/{action}/{git_ref}"),
        urgent: true,
    }
}

/// the machines of an order sorted by how they ended
#[derive(Default)]
struct Tally<'a> {
    /// `switched`, `up to date`, `superseded`, with how many of each
    done: Vec<(&'static str, usize)>,
    /// the hosts that failed and what each says, refusals last
    failures: Vec<(&'a Hostname, String)>,
    refused: usize,
    held: usize,
    asleep: Vec<&'a Hostname>,
    unreachable: Vec<&'a Hostname>,
}

impl<'a> Tally<'a> {
    fn of(action: Action, source: &Source, machines: &'a [Machine]) -> Self {
        let mut tally = Self::default();
        let mut refusals = Vec::new();
        for Machine { host, end } in machines {
            match end {
                End::Finished(Outcome::Succeeded) => tally.count(succeeded(action, source)),
                End::Finished(Outcome::UpToDate) => tally.count("up to date"),
                End::Finished(Outcome::Superseded { .. }) => tally.count("superseded"),
                End::Finished(outcome) => tally.failures.push((host, failure(outcome))),
                End::Rejected(rejection) => {
                    refusals.push((host, format!("refused: {}", describe::rejection(rejection))))
                }
                End::Unreachable(Unreach::Offline(_)) => tally.asleep.push(host),
                End::Unreachable(_) => tally.unreachable.push(host),
                End::Held => tally.held += 1,
            }
        }
        tally.refused = refusals.len();
        tally.failures.extend(refusals);
        tally
    }

    fn count(&mut self, word: &'static str) {
        match self.done.iter_mut().find(|(seen, _)| *seen == word) {
            Some((_, count)) => *count += 1,
            None => self.done.push((word, 1)),
        }
    }

    fn done(&self) -> usize {
        self.done.iter().map(|(_, count)| count).sum()
    }

    fn failed(&self) -> bool {
        !self.failures.is_empty()
    }

    /// `4 switched, 2 up to date in 6m40s`, or `switched in 2m03s` for one
    /// machine; nothing when none got there
    fn counts(&self, total: usize, took: Duration) -> Option<String> {
        let took = format::duration(took);
        match self.done.as_slice() {
            [] => None,
            [(word, _)] if total == 1 => Some(format!("{word} in {took}")),
            done => Some(format!(
                "{} in {took}",
                done.iter()
                    .map(|(word, count)| format!("{count} {word}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
        }
    }

    /// `lab failed evaluating: … · 3 held back · pending: laptop-mac (asleep)`
    fn details(&self) -> Option<String> {
        let mut details: Vec<String> = self
            .failures
            .iter()
            .map(|(host, what)| format!("{host} {what}"))
            .collect();
        if self.held > 0 {
            details.push(format!("{} held back", self.held));
        }
        let pending = [(&self.asleep, "asleep"), (&self.unreachable, "unreachable")]
            .into_iter()
            .filter(|(hosts, _)| !hosts.is_empty())
            .map(|(hosts, why)| format!("{} ({why})", names(hosts)))
            .collect::<Vec<_>>();
        if !pending.is_empty() {
            details.push(format!("pending: {}", pending.join(", ")));
        }
        (!details.is_empty()).then(|| clip(&details.join(" · "), DETAILS))
    }
}

/// what a machine that reached the order's goal did
const fn succeeded(action: Action, source: &Source) -> &'static str {
    match (action, source) {
        (_, Source::Generation(_)) => "rolled back",
        (Action::Switch, _) => "switched",
        (Action::Boot, _) => "set for boot",
        (Action::Test, _) => "activated",
        (Action::Build, _) => "built",
    }
}

/// `failed evaluating: stack overflow`, after the machine's name
fn failure(outcome: &Outcome) -> String {
    let message = |message: &str| clip(message.lines().next().unwrap_or_default(), MESSAGE);
    match outcome {
        Outcome::Failed(failure) => {
            format!("failed {}: {}", failure.phase, message(&failure.message))
        }
        Outcome::RolledBack(failure) => format!(
            "rolled back after {}: {}",
            failure.phase,
            message(&failure.message)
        ),
        Outcome::Cancelled => "was cancelled".into(),
        Outcome::Interrupted => "was interrupted".into(),
        Outcome::Succeeded | Outcome::UpToDate | Outcome::Superseded { .. } => outcome.to_string(),
    }
}

fn title(order: &Order, machines: &[Machine], tally: &Tally<'_>) -> String {
    let source = label(&order.source);
    let deploy = match order.source {
        Source::Generation(_) => format!("Rollback to {source}"),
        _ => format!("Deploy {source}"),
    };
    if tally.failed() {
        let verb = match tally.failures.len() == tally.refused {
            true => "refused by",
            false => "failed on",
        };
        let hosts: Vec<&Hostname> = tally.failures.iter().map(|(host, _)| *host).collect();
        return format!("{deploy} {verb} {}", first(&hosts));
    }
    if tally.done() == 0 {
        let pending: Vec<&Hostname> = tally
            .asleep
            .iter()
            .chain(&tally.unreachable)
            .copied()
            .collect();
        return match pending.is_empty() {
            false => format!("{deploy} pending on {}", first(&pending)),
            true => format!("{deploy} reached no machine"),
        };
    }

    let reach = match machines {
        [only] => match order.action {
            Action::Switch => format!("to {}", only.host),
            _ => format!("on {}", only.host),
        },
        _ => format!("· {} of {}", tally.done(), machines.len()),
    };
    match (&order.source, order.action) {
        (Source::Generation(_), _) => match machines {
            [only] => format!("Rolled {} back to {source}", only.host),
            _ => format!("Rolled back to {source} {reach}"),
        },
        (_, Action::Switch) => format!("Deployed {source} {reach}"),
        (_, Action::Boot) => format!("Staged {source} for boot {reach}"),
        (_, Action::Test) => format!("Activated {source} {reach}"),
        (_, Action::Build) => format!("Built {source} {reach}"),
    }
}

/// `3f2a9c1`, a snapshot's label, `generation 17`
fn label(source: &Source) -> String {
    match source {
        Source::Git { rev, .. } => rev.short(),
        Source::Snapshot { label, .. } => label.clone(),
        Source::Generation(number) => format!("generation {number}"),
    }
}

/// the same for every order of the same thing, so a retried CI run replaces
/// the notice of the run it retries
fn tag(order: &Order, machines: &[Machine]) -> String {
    let action = order.action;
    match &order.source {
        Source::Git { rev, .. } => format!("deploy/{action}/{}", &rev.to_string()[..12]),
        Source::Snapshot { nar_hash, .. } => {
            let digest = nar_hash.file_name();
            let digest = digest.trim_start_matches("sha256-");
            format!("deploy/{action}/{}", &digest[..12.min(digest.len())])
        }
        // a generation's number means something only on its own machine
        Source::Generation(number) => {
            let hosts: Vec<&Hostname> = machines.iter().map(|machine| &machine.host).collect();
            format!("rollback/{}/{number}", names(&hosts))
        }
    }
}

/// `lab`, or `lab +2` when more share its fate
fn first(hosts: &[&Hostname]) -> String {
    match hosts {
        [] => String::new(),
        [only] => only.to_string(),
        [first, rest @ ..] => format!("{first} +{}", rest.len()),
    }
}

fn names(hosts: &[&Hostname]) -> String {
    hosts
        .iter()
        .map(|host| host.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

/// `text`, cut to `max` characters with an ellipsis
fn clip(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        None => text.to_owned(),
        Some(_) => {
            let kept: String = text.chars().take(max - 1).collect();
            format!("{}…", kept.trim_end())
        }
    }
}

/// the subject of `source`'s commit, when the repository at `dir` has it
pub fn subject(dir: &Path, source: &Source) -> Option<String> {
    let Source::Git { rev, .. } = source else {
        return None;
    };
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["show", "--no-patch", "--format=%s"])
        .arg(rev.to_string())
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    let subject = String::from_utf8(output.stdout).ok()?;
    let subject = subject.trim();
    (output.status.success() && !subject.is_empty()).then(|| subject.to_owned())
}

/// composes the notice off the async runtime, since composing may ask git,
/// and sends it
pub async fn announce(settings: PushSettings, notice: impl FnOnce() -> Notice + Send + 'static) {
    let sent =
        tokio::task::spawn_blocking(move || send(&settings, &notice(), SystemTime::now())).await;
    if let Err(error) = sent {
        eprintln!(
            "{}",
            say::warn(format!("sent no push notification: {error}"))
        );
    }
}

/// sends `notice` to every subscribed device and warns about each one it
/// did not reach. it blocks for at most ten seconds, and nothing that goes
/// wrong here is the deploy's failure
pub fn send(settings: &PushSettings, notice: &Notice, at: SystemTime) {
    let warn = |text: String| eprintln!("{}", say::warn(text));
    let push = match Push::load(settings) {
        Ok(push) => push,
        Err(error) => return warn(format!("sent no push notification: {error:#}")),
    };
    let message = match notice.message(push.origin(), at) {
        Ok(message) => message,
        Err(error) => return warn(format!("sent no push notification: {error:#}")),
    };

    for (name, delivery) in push.send(&message) {
        if delivery == Delivery::Delivered {
            continue;
        }
        warn(format!("push to {name}: {delivery}"));
        if let Delivery::Gone { status } = delivery {
            append_step_summary(&format!(
                "> push subscription `{name}` is gone ({status}); remove it from `{}`\n\n",
                shown(&settings.subscriptions_file).display()
            ));
        }
    }
}

/// `path`, relative to the GitHub Actions checkout when it lies inside it
fn shown(path: &Path) -> PathBuf {
    std::env::var_os("GITHUB_WORKSPACE")
        .and_then(|workspace| path.strip_prefix(workspace).ok().map(Path::to_path_buf))
        .unwrap_or_else(|| path.to_path_buf())
}

#[cfg(test)]
mod test;
