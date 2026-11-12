use std::{
    ffi::{OsStr, OsString},
    fmt::Write,
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Output,
};

use eyre::{Result, WrapErr, bail};
use tokio::process::Command;

use crate::common::model::JobId;

/// what the service manager runs on coeusd's behalf
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Task {
    /// the runner of one job
    Job(JobId),
    /// the Full Disk Access probe
    Probe,
}

impl Task {
    /// the systemd unit name
    pub fn unit(self) -> String {
        match self {
            Self::Job(id) => format!("coeus-job-{}", id.0),
            Self::Probe => "coeus-probe".to_owned(),
        }
    }

    /// the launchd label
    pub fn label(self) -> String {
        match self {
            Self::Job(id) => format!("dev.coeus.job.{}", id.0),
            Self::Probe => "dev.coeus.probe".to_owned(),
        }
    }
}

/// starts tasks as transient services of the service manager rather than as
/// coeusd's children, so they outlive coeusd's restarts and self-updates
#[derive(Clone, Debug)]
pub enum Launcher {
    /// `systemd-run`, one collected transient service per task
    Systemd,
    /// `launchctl bootstrap` of a plist kept outside the LaunchDaemons, so it
    /// runs once and never at boot. `program` is the Apple-signed parent the
    /// task runs under: it holds the Full Disk Access grant and spawns the
    /// task as its child, which inherits it.
    Launchd { program: PathBuf, plists: PathBuf },
}

impl Launcher {
    /// starts `command` as `task`; a task already running is left alone
    pub async fn start(&self, task: Task, command: &[OsString], log: &Path) -> Result<()> {
        match self {
            Self::Systemd => {
                let args = systemd_run_args(task, &linux_environment(), command);
                let output = run("systemd-run", &args).await?;
                let stderr = String::from_utf8_lossy(&output.stderr);

                if !output.status.success() && !stderr.contains("already exists") {
                    bail!("systemd-run failed: {}", stderr.trim());
                }
                Ok(())
            }
            Self::Launchd { program, plists } => {
                let label = task.label();
                match LaunchdState::of(&label).await? {
                    LaunchdState::Running => return Ok(()),
                    LaunchdState::Loaded => bootout(&label).await?,
                    LaunchdState::Absent => {}
                }

                let arguments: Vec<OsString> = [program.as_os_str().to_owned()]
                    .into_iter()
                    .chain(command.iter().cloned())
                    .collect();
                let plist = Plist {
                    label: &label,
                    arguments: &arguments,
                    environment: &darwin_environment(),
                    log,
                };

                fs::create_dir_all(plists)?;
                let path = plists.join(format!("{label}.plist"));
                fs::write(&path, plist.render())?;
                fs::set_permissions(&path, fs::Permissions::from_mode(0o644))?;

                let output = run(
                    "launchctl",
                    ["bootstrap", "system"]
                        .map(OsStr::new)
                        .into_iter()
                        .chain([path.as_os_str()]),
                )
                .await?;
                if !output.status.success() {
                    bail!(
                        "launchctl bootstrap failed: {}",
                        String::from_utf8_lossy(&output.stderr).trim()
                    );
                }
                Ok(())
            }
        }
    }

    /// forgets a task that finished. systemd collects its own.
    pub async fn release(&self, task: Task) -> Result<()> {
        match self {
            Self::Systemd => Ok(()),
            Self::Launchd { plists, .. } => {
                let label = task.label();
                if LaunchdState::of(&label).await? != LaunchdState::Absent {
                    bootout(&label).await?;
                }

                match fs::remove_file(plists.join(format!("{label}.plist"))) {
                    Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error.into()),
                    _ => Ok(()),
                }
            }
        }
    }
}

/// `systemd-run` arguments for `command` as the transient service of `task`
pub fn systemd_run_args(
    task: Task,
    environment: &[(String, String)],
    command: &[OsString],
) -> Vec<OsString> {
    let options = [
        format!("--unit={}", task.unit()),
        "--collect".to_owned(),
        "--quiet".to_owned(),
        "--service-type=exec".to_owned(),
    ];
    let environment = environment
        .iter()
        .map(|(name, value)| format!("--setenv={name}={value}"));

    options
        .into_iter()
        .chain(environment)
        .map(OsString::from)
        .chain([OsString::from("--")])
        .chain(command.iter().cloned())
        .collect()
}

/// a launchd job that runs `arguments` once, as root, and leaves the
/// processes it started alone when it exits
pub struct Plist<'a> {
    pub label: &'a str,
    pub arguments: &'a [OsString],
    pub environment: &'a [(String, String)],
    pub log: &'a Path,
}

impl Plist<'_> {
    pub fn render(&self) -> String {
        let mut xml = String::from(concat!(
            r#"<?xml version="1.0" encoding="UTF-8"?>"#,
            "\n",
            r#"<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">"#,
            "\n<plist version=\"1.0\">\n<dict>\n",
        ));

        let log = self.log.to_string_lossy();
        entry(&mut xml, "Label", &string(self.label));
        entry(
            &mut xml,
            "ProgramArguments",
            &array(
                self.arguments
                    .iter()
                    .map(|argument| argument.to_string_lossy()),
            ),
        );
        entry(&mut xml, "RunAtLoad", "<true/>");
        entry(&mut xml, "KeepAlive", "<false/>");
        entry(&mut xml, "AbandonProcessGroup", "<true/>");
        entry(&mut xml, "EnvironmentVariables", &dict(self.environment));
        entry(&mut xml, "StandardOutPath", &string(&log));
        entry(&mut xml, "StandardErrorPath", &string(&log));

        xml.push_str("</dict>\n</plist>\n");
        xml
    }
}

fn entry(xml: &mut String, key: &str, value: &str) {
    let _ = writeln!(xml, "\t<key>{key}</key>\n\t{value}");
}

fn string(text: &str) -> String {
    format!("<string>{}</string>", escape(text))
}

fn array<S: AsRef<str>>(items: impl IntoIterator<Item = S>) -> String {
    let items: String = items
        .into_iter()
        .map(|item| format!("\n\t\t{}", string(item.as_ref())))
        .collect();
    format!("<array>{items}\n\t</array>")
}

fn dict(pairs: &[(String, String)]) -> String {
    let pairs: String = pairs
        .iter()
        .map(|(key, value)| format!("\n\t\t<key>{}</key>\n\t\t{}", escape(key), string(value)))
        .collect();
    format!("<dict>{pairs}\n\t</dict>")
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// what a runner's environment has on NixOS: coeusd's own `PATH`, which the
/// module fills with nix and its tools
fn linux_environment() -> Vec<(String, String)> {
    let inherited = ["PATH", "LOCALE_ARCHIVE"]
        .into_iter()
        .filter_map(|name| Some((name.to_owned(), std::env::var(name).ok()?)));

    inherited
        .chain([
            ("HOME".to_owned(), "/root".to_owned()),
            ("NIX_REMOTE".to_owned(), "daemon".to_owned()),
        ])
        .collect()
}

/// launchd starts jobs with an empty environment; this is what darwin-rebuild
/// runs with as root
fn darwin_environment() -> Vec<(String, String)> {
    let certificates = std::env::var("NIX_SSL_CERT_FILE")
        .unwrap_or_else(|_| "/etc/ssl/certs/ca-certificates.crt".to_owned());

    [
        ("PATH", "/run/current-system/sw/bin:/nix/var/nix/profiles/default/bin:/usr/bin:/bin:/usr/sbin:/sbin".to_owned()),
        ("HOME", "/var/root".to_owned()),
        ("NIX_REMOTE", "daemon".to_owned()),
        ("NIX_SSL_CERT_FILE", certificates),
    ]
    .into_iter()
    .map(|(name, value)| (name.to_owned(), value))
    .collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LaunchdState {
    Absent,
    /// bootstrapped, and either finished or not started yet
    Loaded,
    Running,
}

impl LaunchdState {
    async fn of(label: &str) -> Result<Self> {
        let output = run("launchctl", ["print", &format!("system/{label}")]).await?;
        if !output.status.success() {
            return Ok(Self::Absent);
        }

        let running = String::from_utf8_lossy(&output.stdout)
            .lines()
            .any(|line| line.trim() == "state = running");
        Ok(if running { Self::Running } else { Self::Loaded })
    }
}

async fn bootout(label: &str) -> Result<()> {
    let output = run("launchctl", ["bootout", &format!("system/{label}")]).await?;
    let stderr = String::from_utf8_lossy(&output.stderr);

    // the job may have gone between the check and the bootout
    if !output.status.success()
        && !stderr.contains("No such process")
        && !stderr.contains("Could not find")
    {
        bail!("launchctl bootout failed: {}", stderr.trim());
    }
    Ok(())
}

async fn run<I, A>(program: &str, args: I) -> Result<Output>
where
    I: IntoIterator<Item = A>,
    A: AsRef<OsStr>,
{
    Command::new(program)
        .args(args)
        .output()
        .await
        .wrap_err_with(|| format!("failed to run {program}"))
}
