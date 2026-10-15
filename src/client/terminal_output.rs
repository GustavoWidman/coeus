//! Cancellable local output with temporary output passthrough (no raw input mode).
use eyre::Result;
use nix::{
    fcntl::{FcntlArg, OFlag, fcntl},
    sys::termios::{self, OutputFlags, SetArg, Termios},
};
use std::{
    fs::File,
    io::{IsTerminal, Write},
};
use tokio::io::unix::AsyncFd;

enum OutputFile {
    Ready(AsyncFd<File>),
    Regular(File),
}
impl OutputFile {
    fn file(&self) -> &File {
        match self {
            Self::Ready(fd) => fd.get_ref(),
            Self::Regular(file) => file,
        }
    }
}
pub(super) struct TerminalOutput {
    file: OutputFile,
    flags: OFlag,
    settings: Option<Termios>,
}
impl TerminalOutput {
    pub fn new() -> Result<Self> {
        let file = File::from(nix::unistd::dup(std::io::stdout())?);
        fcntl(&file, FcntlArg::F_SETFD(nix::fcntl::FdFlag::FD_CLOEXEC))?;
        let flags = OFlag::from_bits_truncate(fcntl(&file, FcntlArg::F_GETFL)?);
        let settings = if std::io::stdout().is_terminal() {
            Some(termios::tcgetattr(&file)?)
        } else {
            None
        };
        // Regular files cannot be registered with epoll/kqueue, but never depend
        // on a pipe consumer. TTY/pipe writes are readiness-based/cancellable.
        let stat = nix::sys::stat::fstat(&file)?;
        let regular = nix::sys::stat::SFlag::from_bits_truncate(stat.st_mode)
            .contains(nix::sys::stat::SFlag::S_IFREG);
        let file = if regular {
            OutputFile::Regular(file)
        } else {
            OutputFile::Ready(AsyncFd::new(file)?)
        };
        let result = Self {
            file,
            flags,
            settings,
        };
        fcntl(
            result.file.file(),
            FcntlArg::F_SETFL(flags | OFlag::O_NONBLOCK),
        )?;
        if let Some(settings) = &result.settings {
            let mut passthrough = settings.clone();
            passthrough.output_flags.remove(OutputFlags::OPOST);
            termios::tcsetattr(result.file.file(), SetArg::TCSANOW, &passthrough)?;
        }
        Ok(result)
    }
    pub(super) fn cleanup(&self, bytes: &[u8]) {
        let _ = self.file.file().write_all(bytes);
    }
    pub async fn write(&self, mut bytes: &[u8]) -> Result<()> {
        while !bytes.is_empty() {
            let count = match &self.file {
                OutputFile::Regular(file) => (&*file).write(bytes)?,
                OutputFile::Ready(fd) => {
                    let mut ready = fd.writable().await?;
                    match ready.try_io(|fd| {
                        let mut file = fd.get_ref();
                        file.write(bytes)
                    }) {
                        Ok(result) => result?,
                        Err(_) => continue,
                    }
                }
            };
            eyre::ensure!(count != 0, "terminal output write returned zero");
            bytes = &bytes[count..];
        }
        Ok(())
    }
}
impl Drop for TerminalOutput {
    fn drop(&mut self) {
        let file = self.file.file();
        if let Some(settings) = &self.settings {
            // Best-effort visual cleanup, never blocking while detaching. Restore
            // every termios field even if output is closed/backpressured.
            let _ = (&*file).write_all(b"\x1b[0m\x1b[?25h");
            let _ = termios::tcsetattr(file, SetArg::TCSANOW, settings);
        }
        let _ = fcntl(file, FcntlArg::F_SETFL(self.flags));
    }
}
