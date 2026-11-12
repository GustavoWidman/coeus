//! nix-output-monitor run on the machine that builds, so it reads the
//! derivations it draws from the store that has them, onto a
//! pseudo-terminal of the client's size, like `ssh -tt` gives it

mod feed;
mod pty;

#[cfg(test)]
mod test;

use std::{
    io,
    os::fd::{BorrowedFd, OwnedFd},
    process::{ExitStatus, Stdio},
    sync::Arc,
    thread,
};

use rustix::{io::Errno, termios::Winsize};
use tokio::{
    process::{Child, ChildStdin, Command},
    sync::mpsc,
};

pub(super) use feed::Feed;
use pty::Pty;

use crate::common::proto::{AttachScreen, TerminalSize};

pub(super) struct Nom {
    child: Child,
    master: Arc<OwnedFd>,
    output: mpsc::Receiver<Vec<u8>>,
}

impl Nom {
    /// the package names nom's store path; elsewhere it is found on PATH
    const PROGRAM: &'static str = match option_env!("COEUS_NOM") {
        Some(program) => program,
        None => "nom",
    };

    /// nom reading nix's internal-json from the returned input
    pub fn spawn(size: TerminalSize) -> io::Result<(Self, ChildStdin)> {
        Self::spawn_program(Self::PROGRAM, &["--json"], size)
    }

    fn spawn_program(
        program: &str,
        args: &[&str],
        size: TerminalSize,
    ) -> io::Result<(Self, ChildStdin)> {
        let Pty { master, terminal } = Pty::open()?;
        rustix::termios::tcsetwinsize(&terminal, winsize(size))?;

        let mut command = Command::new(program);
        command
            .args(args)
            .env("TERM", "xterm-256color")
            .stdin(Stdio::piped())
            .stdout(Stdio::from(terminal.try_clone()?))
            .stderr(Stdio::from(terminal))
            .kill_on_drop(true);
        // SAFETY: only async-signal-safe system calls between fork and exec.
        // the terminal becomes nom's controlling one, so a resize reaches it
        // as SIGWINCH
        unsafe {
            command.pre_exec(|| {
                rustix::process::setsid()?;
                rustix::process::ioctl_tiocsctty(BorrowedFd::borrow_raw(1))?;
                Ok(())
            });
        }

        let mut child = command.spawn()?;
        // closes the daemon's copies of the terminal, so the output ends with nom
        drop(command);
        let input = child.stdin.take().expect("nom's stdin is piped");
        let master = Arc::new(master);
        let output = Self::read(master.clone());

        Ok((
            Self {
                child,
                master,
                output,
            },
            input,
        ))
    }

    /// the next piece of what nom drew, at most [AttachScreen::MAX] bytes,
    /// or `None` once nom and its terminal are gone
    pub async fn output(&mut self) -> Option<Vec<u8>> {
        self.output.recv().await
    }

    pub fn resize(&self, size: TerminalSize) -> io::Result<()> {
        Ok(rustix::termios::tcsetwinsize(&*self.master, winsize(size))?)
    }

    pub async fn wait(mut self) -> io::Result<ExitStatus> {
        self.child.wait().await
    }

    /// a thread reads the terminal, which not every platform can poll
    fn read(master: Arc<OwnedFd>) -> mpsc::Receiver<Vec<u8>> {
        let (sender, receiver) = mpsc::channel(16);
        thread::spawn(move || {
            let mut buffer = vec![0; AttachScreen::MAX];
            loop {
                match rustix::io::read(&*master, &mut buffer) {
                    Ok(0) => return,
                    Ok(read) => {
                        if sender.blocking_send(buffer[..read].to_vec()).is_err() {
                            return;
                        }
                    }
                    Err(Errno::INTR) => {}
                    Err(_) => return,
                }
            }
        });
        receiver
    }
}

fn winsize(size: TerminalSize) -> Winsize {
    Winsize {
        ws_row: size.rows,
        ws_col: size.columns,
        ws_xpixel: 0,
        ws_ypixel: 0,
    }
}
