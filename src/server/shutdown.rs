use eyre::Result;

/// Register termination handlers before the server announces readiness.
/// Constructing an async signal future alone does not install its handler.
pub(super) struct Signals {
    #[cfg(unix)]
    terminate: tokio::signal::unix::Signal,
    #[cfg(unix)]
    interrupt: tokio::signal::unix::Signal,
}

impl Signals {
    pub(super) fn register() -> Result<Self> {
        Ok(Self {
            #[cfg(unix)]
            terminate: tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?,
            #[cfg(unix)]
            interrupt: tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?,
        })
    }

    /// Handle both an interactive Ctrl-C and the service manager's termination signal.
    pub(super) async fn wait(&mut self) -> Result<()> {
        #[cfg(unix)]
        {
            let received = tokio::select! {
                received = self.terminate.recv() => received,
                received = self.interrupt.recv() => received,
            };
            eyre::ensure!(received.is_some(), "termination signal stream closed");
        }
        #[cfg(not(unix))]
        tokio::signal::ctrl_c().await?;
        Ok(())
    }
}
