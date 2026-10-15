use std::{
    ffi::{OsStr, OsString},
    process::Stdio,
    time::Duration,
};

use super::{output, process::DeploymentProcess};
use crate::{common::proto::LogSource, jobs::JobContext};
use eyre::{Result, WrapErr, bail};
use log::debug;
use tokio::process::Command;

/// A command and the arguments and environment that will be passed to it.
#[derive(Clone, Debug, Default)]
pub struct CommandRunner {
    program: OsString,
    args: Vec<OsString>,
    env: Vec<(OsString, OsString)>,
    path: Option<OsString>,
    supervised_activation: bool,
}

#[allow(dead_code)] // TODO remove this
impl CommandRunner {
    pub fn new(program: impl Into<OsString>) -> Self {
        Self {
            program: program.into(),
            ..Self::default()
        }
    }

    pub fn path(mut self, path: impl Into<OsString>) -> Self {
        self.path = Some(path.into());
        self
    }

    pub fn arg(mut self, arg: impl Into<OsString>) -> Self {
        self.args.push(arg.into());
        self
    }

    pub fn replace_arg_at(mut self, index: usize, arg: impl Into<OsString>) -> Self {
        self.args[index] = arg.into();
        self
    }

    pub fn args<I, A>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = A>,
        A: AsRef<OsStr>,
    {
        self.args
            .extend(args.into_iter().map(|arg| arg.as_ref().to_os_string()));
        self
    }

    pub fn envs<I, K, V>(mut self, env: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<OsStr>,
        V: AsRef<OsStr>,
    {
        self.env.extend(
            env.into_iter()
                .map(|(key, value)| (key.as_ref().to_os_string(), value.as_ref().to_os_string())),
        );
        self
    }

    /// Root helper owns privileged descendants and uses stdin EOF as cancellation.
    /// This flag is only used with the immutable scoped activation adapter.
    pub(crate) fn supervised_activation(mut self) -> Self {
        self.supervised_activation = true;
        self
    }

    fn command(&self) -> Command {
        let mut command = Command::new(&self.program);
        command
            .args(&self.args)
            .envs(self.env.iter().map(|(key, value)| (key, value)))
            .stdin(if self.supervised_activation {
                Stdio::piped()
            } else {
                Stdio::null()
            });
        if let Some(path) = &self.path {
            command.current_dir(path);
        }
        command
    }

    pub async fn call(self) -> Result<()> {
        let mut process = DeploymentProcess::spawn(&mut self.command())
            .wrap_err_with(|| format!("starting deployment command {:?}", self.program))?;
        let status = process.child.wait().await?;
        if !status.success() {
            bail!("command failed with {status}");
        }
        Ok(())
    }

    /// Independent activation also owns its deadline, including lease revocation
    /// and joined cleanup. An outer timeout must never drop this operation.
    pub(crate) async fn call_with_timeout(self, deadline: Duration) -> Result<()> {
        let mut process = DeploymentProcess::spawn(&mut self.command())
            .wrap_err_with(|| format!("starting deployment command {:?}", self.program))?;
        let lease = process.child.stdin.take();
        let result = tokio::select! {
            _ = tokio::time::sleep(deadline) => Err(eyre::eyre!("activation command deadline exceeded; outcome uncertain")),
            status = process.child.wait() => match status {
                Ok(status) if status.success() => Ok(()),
                Ok(status) => Err(eyre::eyre!("command failed with {status}")),
                Err(error) => Err(error.into()),
            },
        };
        drop(lease);
        if result.is_err() {
            if self.supervised_activation {
                process
                    .terminate_supervised()
                    .await
                    .wrap_err("privileged supervisor did not settle")?;
            } else {
                // Bootstrap/operator sudo has no existing scoped helper. Cleanup
                // may lack privilege; receipt remains Uncertain, never safe/replayed.
                process
                    .terminate()
                    .await
                    .wrap_err("terminating activation command; outcome uncertain")?;
            }
        }
        result
    }

    pub async fn call_with_output(self, context: &JobContext) -> Result<()> {
        self.call_with_deadline(context, Duration::from_secs(3600))
            .await
    }

    #[cfg(not(unix))]
    pub(crate) async fn call_with_deadline(
        self,
        _context: &JobContext,
        _deadline: Duration,
    ) -> Result<()> {
        bail!("terminal-backed deployment commands require Unix");
    }

    #[cfg(unix)]
    pub(crate) async fn call_with_deadline(
        self,
        context: &JobContext,
        deadline: Duration,
    ) -> Result<()> {
        context.check_cancelled()?;
        debug!("starting deployment command");
        let mut command = self.command();
        #[cfg(unix)]
        let reader = super::pty::PtyReader::configure(&mut command, context)?;
        let mut process = DeploymentProcess::spawn(&mut command)
            .wrap_err_with(|| format!("starting deployment command {:?}", self.program))?;
        #[cfg(unix)]
        if let Some(pid) = process.child.id() {
            context
                .terminal
                .lock()
                .map_err(|_| eyre::eyre!("terminal state poisoned"))?
                .process_group(pid);
        }
        // Take stdin out before Child::wait (Tokio otherwise closes it). Only
        // this daemon-side write handle keeps the privileged supervisor alive.
        let lease = process.child.stdin.take();
        // Command owns slave Stdio until it is dropped: retaining it would block EOF.
        drop(command);
        let output = output::drain(reader, LogSource::Terminal, context.clone());
        let execution = async { Ok::<_, eyre::Report>(process.child.wait().await?) };
        let result = tokio::select! {
            biased;
            _ = context.cancelled() => Err(eyre::eyre!("deployment cancellation requested")),
            _ = tokio::time::sleep(deadline) => Err(eyre::eyre!("deployment command deadline exceeded")),
            result = async {
                let (status, ()) = tokio::try_join!(execution, output)?;
                if !status.success() { bail!("command failed with {status}"); }
                Ok(())
            } => result,
        };
        drop(lease);
        if result.is_err() {
            if self.supervised_activation {
                process
                    .terminate_supervised()
                    .await
                    .wrap_err("privileged supervisor did not settle")?;
            } else {
                process
                    .terminate()
                    .await
                    .wrap_err("terminating deployment process group")?;
            }
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn missing_program_identifies_executable_without_dumping_environment() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("coeus-fixture-missing-command");
        let error = CommandRunner::new(&missing)
            .envs([("COEUS_FIXTURE_PRIVATE", "not-for-logs")])
            .call()
            .await
            .unwrap_err();
        let message = format!("{error:#}");
        assert!(message.contains("starting deployment command"));
        assert!(message.contains("coeus-fixture-missing-command"));
        assert!(!message.contains("COEUS_FIXTURE_PRIVATE"));
        assert!(!message.contains("not-for-logs"));
        assert_eq!(
            error.downcast_ref::<std::io::Error>().unwrap().kind(),
            std::io::ErrorKind::NotFound
        );
    }

    #[tokio::test]
    async fn path_sets_the_child_working_directory() -> Result<()> {
        let dir = tempfile::tempdir()?;
        std::fs::write(dir.path().join("marker"), [])?;

        CommandRunner::new("sh")
            .path(dir.path())
            .args(["-c", "test -f marker", "sh"])
            .call()
            .await
    }
    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn independent_activation_timeout_joins_supervisor_cleanup() -> Result<()> {
        tokio::time::timeout(Duration::from_secs(5), async {
            let temp = tempfile::tempdir()?;
            let marker = temp.path().join("child.pid");
            let code = r#"import importlib.util,os,sys,time
s=importlib.util.spec_from_file_location('p',sys.argv[1]); p=importlib.util.module_from_spec(s); s.loader.exec_module(p)
code=p.supervised(['/bin/sh','-c','sleep 30 & echo $! > "$1"; wait','sh',sys.argv[2]],dict(os.environ),deadline=10)
time.sleep(0.15)
open(sys.argv[2]+'.settled','w').write(str(code))
sys.exit(code)"#;
            let error = CommandRunner::new("python3").arg("-c").arg(code)
                .arg(concat!(env!("CARGO_MANIFEST_DIR"), "/nix/elevation.py"))
                .arg(&marker).supervised_activation()
                .call_with_timeout(Duration::from_millis(700)).await.unwrap_err();
            assert!(format!("{error:#}").contains("deadline"));
            assert!(marker.with_extension("pid.settled").is_file());
            let pid: u32 = std::fs::read_to_string(&marker)?.trim().parse()?;
            match tokio::fs::read_to_string(format!("/proc/{pid}/stat")).await {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
                Ok(stat) if stat.split_once(") ").is_some_and(|(_, rest)| rest.starts_with('Z')) => {},
                other => panic!("supervised child survived independent timeout: {other:?}"),
            }
            Ok::<_, eyre::Report>(())
        }).await??;
        Ok(())
    }
}
