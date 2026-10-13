use eyre::Result;
use tokio::process::{Child, Command};

/// Own the child's process group, not just the `nh` parent. Cancellation must also
/// reap Nix/build descendants that inherited the output pipes.
pub(super) struct DeploymentProcess {
    pub child: Child,
    #[cfg(unix)]
    group: Option<nix::unistd::Pid>,
}

impl DeploymentProcess {
    pub fn spawn(command: &mut Command) -> Result<Self> {
        command.kill_on_drop(true);
        #[cfg(unix)]
        command.process_group(0);
        let child = command.spawn()?;
        #[cfg(unix)]
        let group = child.id().map(|id| nix::unistd::Pid::from_raw(id as i32));
        Ok(Self {
            child,
            #[cfg(unix)]
            group,
        })
    }
    pub async fn terminate(&mut self) -> Result<()> {
        let group_result = self.kill_group();
        // Still attempt/reap the parent if signalling a group fails.
        let child_result = self.child.kill().await;
        group_result?;
        child_result?;
        Ok(())
    }
    fn kill_group(&self) -> Result<()> {
        #[cfg(unix)]
        if let Some(group) = self.group {
            match nix::sys::signal::killpg(group, nix::sys::signal::Signal::SIGKILL) {
                Ok(()) | Err(nix::errno::Errno::ESRCH) => {}
                Err(error) => return Err(error.into()),
            }
        }
        Ok(())
    }
}

impl Drop for DeploymentProcess {
    fn drop(&mut self) {
        if let Err(error) = self.kill_group() {
            log::error!("failed to terminate deployment process group: {error}");
        }
    }
}
