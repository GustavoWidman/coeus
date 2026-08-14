use std::ffi::{OsStr, OsString};

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

    pub async fn call(self) -> Result<()> {
        let Self {
            program,
            args,
            env,
            path,
        } = self;
        let program_name = program.to_string_lossy();
        debug!("running {program_name}");

        let mut command = Command::new(&program);
        command
            .args(&args)
            .envs(env.iter().map(|(key, value)| (key, value)));

        if let Some(path) = path {
            command.env("PATH", path);
        }

        let status = command
            .status()
            .await
            .wrap_err_with(|| format!("failed to start {program_name}"))?;

        if !status.success() {
            bail!("{program_name} failed with {status}");
        }

        Ok(())
    }
}
