use log::warn;
use tokio::process::Command;

use crate::{
    common::model::{Hostname, JobState, JobSummary},
    config::{NotifyOn, NotifySettings},
};

/// announces finished jobs to an ntfy topic or any url taking a text body
pub(super) struct Notifier {
    settings: NotifySettings,
}

impl Notifier {
    pub const fn new(settings: NotifySettings) -> Self {
        Self { settings }
    }

    pub fn announce(&self, host: &Hostname, job: &JobSummary) {
        let JobState::Finished(outcome) = &job.state else {
            return;
        };

        if outcome.is_success() && self.settings.on == NotifyOn::Failure {
            return;
        }

        let verdict = match outcome.failure() {
            Some(failure) => format!("failed {}: {}", failure.phase, failure.message),
            None if outcome.is_success() => "succeeded".to_owned(),
            None => format!("{outcome:?}").to_lowercase(),
        };
        let title = format!("{host} {} {verdict}", job.id);
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
            .args(["--data", &body, &self.settings.url]);

        tokio::spawn(async move {
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
}
