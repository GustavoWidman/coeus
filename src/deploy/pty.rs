//! Safe output-only PTY setup: no shell/input forwarding or new controlling TTY.
//! Existing daemon session/controlling-terminal inheritance is unchanged.
use crate::jobs::JobContext;
use eyre::{Result, WrapErr};
use std::{
    fs::File,
    io::{self, Read},
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};
use tokio::{
    io::{AsyncRead, ReadBuf, unix::AsyncFd},
    process::Command,
};

pub(super) struct PtyReader {
    master: AsyncFd<File>,
    context: JobContext,
}
impl PtyReader {
    pub fn configure(command: &mut Command, context: &JobContext) -> Result<Self> {
        let pair = nix::pty::openpty(None, None).wrap_err("opening deployment PTY")?;
        // Neither master nor original slave may leak through exec. Only the
        // child's explicitly duplicated stdout/stderr descriptors remain open.
        for fd in [&pair.master, &pair.slave] {
            nix::fcntl::fcntl(
                fd,
                nix::fcntl::FcntlArg::F_SETFD(nix::fcntl::FdFlag::FD_CLOEXEC),
            )?;
        }
        // Master reads are readiness-based, never detached blocking reader tasks.
        nix::fcntl::fcntl(
            &pair.master,
            nix::fcntl::FcntlArg::F_SETFL(nix::fcntl::OFlag::O_NONBLOCK),
        )?;
        let master = File::from(pair.master);
        let slave = File::from(pair.slave);
        command
            .stdout(slave.try_clone()?)
            .stderr(slave)
            .env("TERM", "xterm-256color");
        let resize = Arc::new(master.try_clone()?);
        context
            .terminal
            .lock()
            .map_err(|_| eyre::eyre!("terminal state poisoned"))?
            .register(resize)?;
        let result = AsyncFd::new(master);
        if result.is_err() {
            context
                .terminal
                .lock()
                .map_err(|_| eyre::eyre!("terminal state poisoned"))?
                .unregister();
        }
        Ok(Self {
            master: result?,
            context: context.clone(),
        })
    }
}
impl Drop for PtyReader {
    fn drop(&mut self) {
        if let Ok(mut terminal) = self.context.terminal.lock() {
            terminal.unregister();
        }
    }
}
impl AsyncRead for PtyReader {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        loop {
            let mut ready = std::task::ready!(self.master.poll_read_ready(cx))?;
            let result = ready.try_io(|inner| {
                let mut file = inner.get_ref();
                file.read(buffer.initialize_unfilled())
            });
            match result {
                Err(_) => continue,
                Ok(Ok(count)) => {
                    buffer.advance(count);
                    return Poll::Ready(Ok(()));
                }
                // Linux returns EIO when the final slave closes; Darwin returns EOF.
                Ok(Err(error)) if error.raw_os_error() == Some(nix::libc::EIO) => {
                    return Poll::Ready(Ok(()));
                }
                Ok(Err(error)) => return Poll::Ready(Err(error)),
            }
        }
    }
}
