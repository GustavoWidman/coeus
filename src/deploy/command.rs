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

    fn command(&self) -> Command {
        let mut command = Command::new(&self.program);
        command
            .args(&self.args)
            .envs(self.env.iter().map(|(key, value)| (key, value)))
            .stdin(Stdio::null());
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
        if result.is_err() {
            process
                .terminate()
                .await
                .wrap_err("terminating deployment process group")?;
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
}
