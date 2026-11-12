use serde::Serialize;

use crate::common::model::{
    DaemonState, FdaStatus, Hostname, JobState, MachineView, Outcome, Platform, Source,
};

/// what `coeus doctor` finds on one machine
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Checkup {
    pub host: Hostname,
    pub findings: Vec<Finding>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub enum Severity {
    Ok,
    Warn,
    Fail,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Finding {
    pub severity: Severity,
    pub title: String,
    /// what to do about it, as a command or a place to click
    pub fix: Option<String>,
}

impl Finding {
    fn new(severity: Severity, title: impl Into<String>) -> Self {
        Self {
            severity,
            title: title.into(),
            fix: None,
        }
    }

    fn fix(mut self, fix: impl Into<String>) -> Self {
        self.fix = Some(fix.into());
        self
    }
}

impl Checkup {
    /// the path System Settings asks for when adding a Full Disk Access entry
    pub const LAUNCHER: &'static str = "/usr/bin/time";

    pub fn unreachable(host: Hostname, error: &str) -> Self {
        Self {
            findings: vec![
                Finding::new(Severity::Fail, format!("unreachable: {error}")).fix(format!(
                    "check that {host} is online, on the tailnet and runs coeusd"
                )),
            ],
            host,
        }
    }

    /// everything worth knowing about a machine, worst first
    pub fn of(view: &MachineView, cli_version: &str) -> Self {
        let host = &view.host;
        let system = &view.system;
        let mut findings = Vec::new();

        if host.version != cli_version {
            findings.push(Finding::new(
                Severity::Warn,
                format!(
                    "runs coeusd {} while this coeus is {cli_version}",
                    host.version
                ),
            ));
        }

        if view.daemon == DaemonState::Restarting {
            findings.push(Finding::new(
                Severity::Warn,
                "coeusd is restarting into a newer version",
            ));
        }

        if host.platform == Platform::Darwin && system.fda == FdaStatus::Missing {
            findings.push(
                Finding::new(
                    Severity::Fail,
                    "cannot activate: Full Disk Access is missing",
                )
                .fix(format!(
                    "grant it to {} in System Settings > Privacy & Security > Full Disk Access, or run `coeus doctor --fix` on {}",
                    Self::LAUNCHER,
                    host.hostname
                )),
            );
        }

        if system.current.is_none() {
            findings.push(Finding::new(Severity::Fail, "no running system found"));
        }

        if system.reboot_required {
            findings.push(
                Finding::new(Severity::Warn, "runs a kernel older than its system")
                    .fix(format!("coeus reboot {}", host.hostname)),
            );
        } else if system.pending_boot {
            findings.push(
                Finding::new(Severity::Warn, "a boot deployment waits for a reboot")
                    .fix(format!("coeus reboot {}", host.hostname)),
            );
        }

        if let Some(follow) = &system.follow {
            match (&follow.latest, follow.paused) {
                (_, true) => findings.push(Finding::new(
                    Severity::Warn,
                    format!(
                        "holds {} until {} moves",
                        describe(system.deployed.as_ref()),
                        follow.git_ref
                    ),
                )),
                (Some(latest), false) if !matches!(&system.deployed, Some(Source::Git { rev, .. }) if *rev == latest.rev) => {
                    findings.push(Finding::new(
                        Severity::Warn,
                        format!(
                            "behind {} ({}), catching up on its own",
                            follow.git_ref,
                            latest.rev.short()
                        ),
                    ))
                }
                _ => {}
            }
        }

        if let Some(last) = view.recent.first()
            && let JobState::Finished(outcome) = &last.state
            && !outcome.is_success()
        {
            let title = match outcome.failure() {
                Some(failure) => format!(
                    "job {} failed {}: {}",
                    last.id, failure.phase, failure.message
                ),
                None if *outcome == Outcome::Interrupted => {
                    format!("job {} was interrupted", last.id)
                }
                None => format!("job {} was cancelled", last.id),
            };
            findings.push(
                Finding::new(Severity::Warn, title)
                    .fix(format!("coeus attach {} {}", host.hostname, last.id.0)),
            );
        }

        if findings.is_empty() {
            findings.push(Finding::new(
                Severity::Ok,
                format!("healthy, runs {}", describe(system.deployed.as_ref())),
            ));
        }

        findings.sort_by(|a, b| b.severity.cmp(&a.severity));
        Self {
            host: host.hostname.clone(),
            findings,
        }
    }

    pub fn severity(&self) -> Severity {
        self.findings
            .iter()
            .map(|finding| finding.severity)
            .max()
            .unwrap_or(Severity::Ok)
    }
}

fn describe(source: Option<&Source>) -> String {
    match source {
        Some(source) => source.to_string(),
        None => "a system coeus did not deploy".into(),
    }
}
