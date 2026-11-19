use std::{
    collections::VecDeque,
    time::{Duration, SystemTime},
};

use log::{info, warn};
use tokio::{process::Command, task::JoinSet};

use super::tries::{self, Retry};
use crate::{
    common::model::{Hostname, JobState, JobSummary, Origin, Outcome, Source, Timestamp},
    config::{DaemonSettings, NotifyOn, NotifySettings, PushSettings},
    push::{Delivery, Notice, Push},
    ui::format,
};

/// announces finished jobs to an ntfy topic or any url taking a text body,
/// and catch-ups to the subscribed devices by Web Push
pub(super) struct Notifier {
    ntfy: Option<NotifySettings>,
    push: Option<PushSettings>,
    /// sends still in flight, and finished ones not reaped yet
    sends: JoinSet<()>,
}

impl Notifier {
    pub fn new(settings: &DaemonSettings) -> Self {
        Self {
            ntfy: settings.notify.clone(),
            push: settings.push.clone(),
            sends: JoinSet::new(),
        }
    }

    /// announces `job`, which just finished. `recent` holds every finished
    /// job, newest first, `job` included
    pub fn announce(&mut self, host: &Hostname, job: &JobSummary, recent: &VecDeque<JobSummary>) {
        while self.sends.try_join_next().is_some() {}

        if let Some(ntfy) = &self.ntfy
            && let Some(mut command) = ntfy_command(ntfy, host, job)
        {
            self.sends.spawn(async move {
                match command.output().await {
                    Ok(output) if output.status.success() => {}
                    Ok(output) => warn!(
                        "notification failed: {}",
                        String::from_utf8_lossy(&output.stderr).trim()
                    ),
                    Err(error) => warn!("notification failed: {error}"),
                }
            });
        }

        if let Some(push) = &self.push
            && let Some(notice) = catch_up(host, job, recent)
        {
            let push = push.clone();
            let at = job.finished_at.unwrap_or(job.ordered_at);
            self.sends.spawn_blocking(move || send(&push, &notice, at));
        }
    }
}

fn ntfy_command(settings: &NotifySettings, host: &Hostname, job: &JobSummary) -> Option<Command> {
    let JobState::Finished(outcome) = &job.state else {
        return None;
    };

    if outcome.is_success() && settings.on == NotifyOn::Failure {
        return None;
    }

    let title = format!("{host} {} {outcome}", job.id);
    let body = format!("{} {} ordered by {}", job.action, job.source, job.origin);
    let priority = match outcome.is_success() {
        true => "default",
        false => "high",
    };

    let mut command = Command::new("curl");
    command
        .args(["--silent", "--show-error", "--fail", "--max-time", "10"])
        .args(["--header", &format!("Title: {title}")])
        .args(["--header", &format!("Priority: {priority}")])
        .args(["--data", &body, &settings.url]);
    Some(command)
}

/// the most of a failure's message a notice quotes
const QUOTE: usize = 80;

/// the notice for a finished job, if any. only jobs the machine ordered
/// itself to catch up speak, as a deploy's jobs are its summary's business.
/// a success always does. a failure does only when it ends the tries of its
/// commit, by the rule catching up keeps, so one that the next try may
/// still fix says nothing. a cancelled or superseded job was someone's doing
pub(super) fn catch_up(
    host: &Hostname,
    job: &JobSummary,
    recent: &VecDeque<JobSummary>,
) -> Option<Notice> {
    let JobState::Finished(outcome) = &job.state else {
        return None;
    };
    if job.origin != Origin::Follow {
        return None;
    }

    let commit = match &job.source {
        Source::Git { rev, .. } => rev.short(),
        other => other.to_string(),
    };
    let (title, body, urgent) = match outcome {
        Outcome::Succeeded => {
            let took = job
                .started_at
                .zip(job.finished_at)
                .map(|(started, finished)| format::duration(started.until(finished)));
            let body = match took {
                Some(took) => format!("switched in {took}"),
                None => "switched".to_owned(),
            };
            (format!("{host} caught up to {commit}"), body, false)
        }
        Outcome::UpToDate => (
            format!("{host} caught up to {commit}"),
            "already ran its system".to_owned(),
            false,
        ),
        Outcome::Cancelled | Outcome::Superseded { .. } => return None,
        Outcome::Failed(_) | Outcome::RolledBack(_) | Outcome::Interrupted => {
            if tries::retry(recent, &job.source) != Retry::Never {
                return None;
            }
            let again = match &job.source {
                Source::Git {
                    git_ref: Some(git_ref),
                    ..
                } => format!("tries again when {git_ref} moves"),
                _ => "tries again when the ref moves".to_owned(),
            };
            (
                format!("{host} stopped catching up to {commit}"),
                format!("{}\n{again}", failure(outcome)),
                true,
            )
        }
    };

    Some(Notice {
        title,
        body,
        path: format!("machines/{host}/jobs/{}", job.id.0),
        tag: format!("catch-up/{host}"),
        urgent,
    })
}

/// `failed building: …`, or `failed fetching 3 times: …` for the failures
/// catching up tries again
fn failure(outcome: &Outcome) -> String {
    match outcome {
        Outcome::Failed(failure) if tries::passing(outcome) => format!(
            "failed {} {} times: {}",
            failure.phase,
            tries::LIMIT,
            quote(&failure.message)
        ),
        Outcome::Failed(failure) => {
            format!("failed {}: {}", failure.phase, quote(&failure.message))
        }
        Outcome::RolledBack(failure) => format!(
            "rolled back after {}: {}",
            failure.phase,
            quote(&failure.message)
        ),
        _ => "interrupted while activating".to_owned(),
    }
}

/// the first line of `message`, cut to [QUOTE] characters
fn quote(message: &str) -> String {
    let line = message.lines().next().unwrap_or_default().trim();
    match line.char_indices().nth(QUOTE) {
        Some((end, _)) => format!("{}…", line[..end].trim_end()),
        None => line.to_owned(),
    }
}

/// sends `notice`, about a job that finished `at`, to every subscribed
/// device and logs how each went. it blocks for at most ten seconds
fn send(settings: &PushSettings, notice: &Notice, at: Timestamp) {
    let at = SystemTime::UNIX_EPOCH + Duration::from_millis(at.0.max(0).cast_unsigned());
    let message = Push::load(settings).and_then(|push| {
        let message = notice.message(push.origin(), at)?;
        Ok((push, message))
    });
    let (push, message) = match message {
        Ok(loaded) => loaded,
        Err(error) => {
            warn!("cannot push \"{}\": {error:#}", notice.title);
            return;
        }
    };

    for (device, delivery) in push.send(&message) {
        match delivery {
            Delivery::Delivered => info!("pushed \"{}\" to {device}", notice.title),
            delivery => warn!("push of \"{}\" to {device}: {delivery}", notice.title),
        }
    }
}
