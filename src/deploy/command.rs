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
    ssh: Option<SshOptions>,
    path: Option<OsString>,
}

#[derive(Clone, Debug)]
struct SshOptions {
    remote: OsString,
    address: Option<OsString>,
    key: Option<OsString>,
}

#[allow(dead_code)] // TODO remove this
impl CommandRunner {
    pub fn new(program: impl Into<OsString>) -> Self {
        Self {
            program: program.into(),
            ..Self::default()
        }
    }

    pub fn ssh(mut self, remote: &str, address: Option<&str>, key: Option<&str>) -> Self {
        self.ssh = Some(SshOptions {
            remote: remote.into(),
            address: address.map(Into::into),
            key: key.map(Into::into),
        });
        self
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
            ssh,
            path,
        } = self;
        let program_name = program.to_string_lossy();
        debug!("running {program_name}");

        let mut command = match ssh {
            Some(ssh) => {
                let mut command = Command::new("ssh");
                command.args(["-o", "StrictHostKeyChecking=no"]);
                command.args(["-o", "ConnectTimeout=10"]);
                if let Some(address) = ssh.address {
                    command
                        .arg("-o")
                        .arg(format!("HostName={}", address.to_string_lossy()));
                }
                if let Some(key) = ssh.key {
                    command.arg("-i").arg(key);
                }
                let remote_command = Self::remote_command(&program, &args, &env, path.as_deref())?;
                command.arg(ssh.remote).arg(remote_command);
                command
            }
            None => {
                let mut command = Command::new(&program);
                command
                    .args(&args)
                    .envs(env.iter().map(|(key, value)| (key, value)));
                if let Some(path) = path {
                    command.env("PATH", path);
                }
                command
            }
        };

        let status = command
            .status()
            .await
            .wrap_err_with(|| format!("failed to start {program_name}"))?;

        if !status.success() {
            bail!("{program_name} failed with {status}");
        }

        Ok(())
    }

    fn remote_command(
        program: &OsStr,
        args: &[OsString],
        env: &[(OsString, OsString)],
        path: Option<&OsStr>,
    ) -> Result<String> {
        // SSH hands the command to the remote user's shell. Command names must
        // remain bare for Nushell, while arguments still need shell quoting.
        let mut command = if env.is_empty() && path.is_none() {
            String::new()
        } else {
            String::from("env --")
        };

        for (key, value) in env {
            let mut assignment = key.clone();
            assignment.push("=");
            assignment.push(value);
            Self::push_shell_word(&mut command, &assignment)?;
        }

        if let Some(path) = path {
            let mut assignment = OsString::from("PATH=");
            assignment.push(path);
            Self::push_shell_word(&mut command, &assignment)?;
        }

        Self::push_program(&mut command, program)?;
        for arg in args {
            Self::push_shell_word(&mut command, arg)?;
        }

        Ok(command)
    }

    fn push_program(command: &mut String, program: &OsStr) -> Result<()> {
        let program = program
            .to_str()
            .ok_or_else(|| eyre::eyre!("remote programs must contain valid UTF-8"))?;

        if program.is_empty()
            || !program.chars().all(|character| {
                character.is_ascii_alphanumeric()
                    || matches!(character, '_' | '-' | '.' | '/' | ':' | '+')
            })
        {
            bail!("remote program contains unsupported shell characters: {program:?}");
        }

        if !command.is_empty() {
            command.push(' ');
        }
        command.push_str(program);

        Ok(())
    }

    fn push_shell_word(command: &mut String, word: &OsStr) -> Result<()> {
        let word = word
            .to_str()
            .ok_or_else(|| eyre::eyre!("remote commands must contain valid UTF-8"))?;

        if !command.is_empty() {
            command.push(' ');
        }
        command.push('\'');
        command.push_str(&word.replace('\'', "'\"'\"'"));
        command.push('\'');

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::CommandRunner;
    use std::ffi::{OsStr, OsString};

    #[test]
    fn remote_command_passes_environment_and_path_to_remote_process() {
        let args = vec![OsString::from("it's here")];
        let env = vec![(OsString::from("GREETING"), OsString::from("hello world"))];

        let command = CommandRunner::remote_command(
            OsStr::new("printenv"),
            &args,
            &env,
            Some(OsStr::new("/custom path")),
        )
        .unwrap();

        assert_eq!(
            command,
            r#"env -- 'GREETING=hello world' 'PATH=/custom path' printenv 'it'"'"'s here'"#
        );
    }

    #[test]
    fn remote_command_does_not_add_env_when_no_environment_is_configured() {
        let args = vec![OsString::from("hello world")];

        let command =
            CommandRunner::remote_command(OsStr::new("printf"), &args, &[], None).unwrap();

        assert_eq!(command, "printf 'hello world'");
    }
}
