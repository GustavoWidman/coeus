use std::{
    io,
    process::{ExitStatus, Stdio},
    time::Duration,
};

use eyre::{Result, eyre};
use rustix::process::{Pid, Signal, kill_process_group};
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, BufReader},
    process::{Child, Command},
    time,
};

use super::feed::Feed;

/// where a process's stdout goes
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stdout {
    /// kept for the caller, as nix's JSON answers are
    Capture,
    /// into the job log with stderr, as activation output is
    Log,
}

/// a child in its own process group, so stopping it also stops whatever it
/// started: nix's fetchers and substituters, or an activation's helpers.
/// dropping it before it exits kills the group.
pub struct Process {
    child: Child,
    group: Option<Pid>,
}

impl Process {
    /// how often output that is still flowing reaches the log's readers
    const FLUSH: Duration = Duration::from_millis(100);
    /// how long output may keep flowing after the process exited
    const LINGER: Duration = Duration::from_secs(2);

    pub fn spawn(mut command: Command) -> Result<Self> {
        let program = command
            .as_std()
            .get_program()
            .to_string_lossy()
            .into_owned();
        let child = command
            .process_group(0)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| eyre!("failed to start {program}: {error}"))?;
        let group = child
            .id()
            .and_then(|id| Pid::from_raw(i32::try_from(id).ok()?));

        Ok(Self { child, group })
    }

    /// feeds the output to `feed` line by line until the process exits
    pub async fn finish(mut self, feed: &mut Feed<'_>, stdout: Stdout) -> Result<Exit> {
        let (Some(out), Some(err)) = (self.child.stdout.take(), self.child.stderr.take()) else {
            return Err(eyre!("the process has no output pipes"));
        };
        let mut out = Lines::new(out);
        let mut err = Lines::new(err);
        let mut captured = Vec::new();
        let mut flush = time::interval(Self::FLUSH);
        let mut status = None;
        let mut linger = None;

        while out.open || err.open {
            tokio::select! {
                line = err.next(), if err.open => {
                    if let Some(line) = line? {
                        feed.line(&line)?;
                    }
                }
                line = out.next(), if out.open => match (line?, stdout) {
                    (Some(line), Stdout::Capture) => {
                        captured.extend_from_slice(line.as_bytes());
                        captured.push(b'\n');
                    }
                    (Some(line), Stdout::Log) => feed.line(&line)?,
                    (None, _) => {}
                },
                exit = self.child.wait(), if status.is_none() => {
                    status = Some(exit?);
                    linger = Some(time::Instant::now() + Self::LINGER);
                }
                _ = flush.tick() => {
                    feed.tick()?;
                    // something the process started may hold its pipes open
                    if linger.is_some_and(|until| time::Instant::now() >= until) {
                        break;
                    }
                }
            }
        }

        let status = match status {
            Some(status) => status,
            None => self.child.wait().await?,
        };
        self.group = None;
        feed.settle()?;

        Ok(Exit {
            status,
            stdout: captured,
        })
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        if let Some(group) = self.group {
            let _ = kill_process_group(group, Signal::KILL);
        }
    }
}

pub struct Exit {
    pub status: ExitStatus,
    pub stdout: Vec<u8>,
}

/// the lines of a pipe, lossily decoded
struct Lines<R> {
    reader: BufReader<R>,
    /// the line being read, kept whole across cancelled reads
    line: Vec<u8>,
    open: bool,
}

impl<R: AsyncRead + Unpin> Lines<R> {
    fn new(reader: R) -> Self {
        Self {
            reader: BufReader::new(reader),
            line: Vec::new(),
            open: true,
        }
    }

    async fn next(&mut self) -> io::Result<Option<String>> {
        let read = self.reader.read_until(b'\n', &mut self.line).await?;
        if read == 0 {
            self.open = false;
            if self.line.is_empty() {
                return Ok(None);
            }
        }

        let line = String::from_utf8_lossy(&self.line)
            .trim_end_matches(['\n', '\r'])
            .to_owned();
        self.line.clear();
        Ok(Some(line))
    }
}
