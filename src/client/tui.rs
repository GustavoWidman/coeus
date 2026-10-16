//! Structured fleet view. Raw per-job/shared attachment is a separate byte stream.
use super::{
    fleet::AcceptedTarget,
    presentation::{self, DisplayRow, Size, Style, Tone},
};
use crate::common::proto::*;
use eyre::{Result, bail};
use std::time::Duration;

struct Row {
    display: DisplayRow,
    done: bool,
    succeeded: bool,
}

struct Update {
    index: usize,
    result: Result<JobStatus>,
}

#[cfg(unix)]
struct Input {
    fd: tokio::io::unix::AsyncFd<std::fs::File>,
    settings: nix::sys::termios::Termios,
    flags: nix::fcntl::OFlag,
}
#[cfg(unix)]
impl Input {
    fn new() -> Result<Self> {
        use nix::{
            fcntl::{FcntlArg, OFlag, fcntl},
            sys::termios::{self, LocalFlags, SetArg, SpecialCharacterIndices},
        };
        let file = std::fs::File::from(nix::unistd::dup(std::io::stdin())?);
        fcntl(&file, FcntlArg::F_SETFD(nix::fcntl::FdFlag::FD_CLOEXEC))?;
        let settings = termios::tcgetattr(&file)?;
        let flags = OFlag::from_bits_truncate(fcntl(&file, FcntlArg::F_GETFL)?);
        let input = Self {
            fd: tokio::io::unix::AsyncFd::new(file)?,
            settings,
            flags,
        };
        let mut keys = input.settings.clone();
        keys.local_flags
            .remove(LocalFlags::ICANON | LocalFlags::ECHO);
        // Keep ISIG: Ctrl+C detaches the client; no input reaches a deployment.
        keys.control_chars[SpecialCharacterIndices::VMIN as usize] = 1;
        keys.control_chars[SpecialCharacterIndices::VTIME as usize] = 0;
        termios::tcsetattr(input.fd.get_ref(), SetArg::TCSANOW, &keys)?;
        fcntl(
            input.fd.get_ref(),
            FcntlArg::F_SETFL(flags | OFlag::O_NONBLOCK),
        )?;
        Ok(input)
    }
    async fn read(&self) -> Result<Vec<u8>> {
        use std::io::Read;
        loop {
            let mut ready = self.fd.readable().await?;
            let mut bytes = [0; 64];
            match ready.try_io(|fd| fd.get_ref().read(&mut bytes)) {
                Ok(result) => {
                    let count = result?;
                    return Ok(bytes[..count].to_vec());
                }
                Err(_) => continue,
            }
        }
    }
}
#[cfg(unix)]
impl Drop for Input {
    fn drop(&mut self) {
        use nix::{
            fcntl::{FcntlArg, fcntl},
            sys::termios::{self, SetArg},
        };
        let _ = termios::tcsetattr(self.fd.get_ref(), SetArg::TCSANOW, &self.settings);
        let _ = fcntl(self.fd.get_ref(), FcntlArg::F_SETFL(self.flags));
    }
}

#[cfg(unix)]
struct Screen {
    // Input captures original modes before output passthrough. Drop output first,
    // then input, so the last restore also restores the original input modes.
    input: Input,
    output: Option<super::terminal_output::TerminalOutput>,
}
#[cfg(unix)]
impl Screen {
    fn new() -> Result<Self> {
        let input = Input::new()?;
        let output = super::terminal_output::TerminalOutput::new()?;
        Ok(Self {
            input,
            output: Some(output),
        })
    }
    async fn write(&self, bytes: &[u8]) -> Result<()> {
        if let Some(output) = &self.output {
            output.write(bytes).await?;
        }
        Ok(())
    }
}
#[cfg(unix)]
impl Drop for Screen {
    fn drop(&mut self) {
        if let Some(output) = self.output.take() {
            output.cleanup(b"\x1b[0m\x1b[?25h\x1b[?1049l");
            drop(output);
        }
    }
}

#[cfg(unix)]
pub(super) async fn follow(targets: Vec<AcceptedTarget>, failures: &[DisplayRow]) -> Result<()> {
    let mut interrupt = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
    let screen = Screen::new()?;
    tokio::select! { biased; _ = interrupt.recv() => return Ok(()), result = screen.write(b"\x1b[?1049h\x1b[?25l") => result? }
    let style = Style::stdout();
    let (sender, mut updates) = tokio::sync::mpsc::channel(128);
    let limit = std::sync::Arc::new(tokio::sync::Semaphore::new(8));
    let mut tasks = tokio::task::JoinSet::new();
    let mut rows = Vec::new();
    for (index, mut target) in targets.into_iter().enumerate() {
        rows.push(Row {
            display: DisplayRow::pending(&target.name, target.job_id),
            done: false,
            succeeded: false,
        });
        let sender = sender.clone();
        let limit = limit.clone();
        tasks.spawn(async move {
            loop {
                let result = async {
                    let _permit = limit.acquire().await?;
                    let Packet::StatusResponse(response) = target
                        .client
                        .request(StatusRequest {
                            job_id: Some(target.job_id),
                            after: None,
                        })
                        .await?
                    else {
                        bail!("unexpected job status response");
                    };
                    response
                        .jobs
                        .into_iter()
                        .find(|job| job.job_id == target.job_id)
                        .ok_or_else(|| eyre::eyre!("accepted job missing from status"))
                }
                .await;
                let done = result.as_ref().map_or(true, |job| job.phase.is_terminal());
                if sender.send(Update { index, result }).await.is_err() || done {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(300)).await;
            }
        });
    }
    drop(sender);
    for failure in failures {
        rows.push(Row {
            display: failure.clone(),
            done: true,
            succeeded: false,
        });
    }
    let mut offset = 0;
    let mut last_frame = String::new();
    let mut resize = tokio::time::interval(Duration::from_millis(150));
    let mut keys = Keys::default();
    loop {
        let size = Size::stdout();
        let frame = render(&rows, size, style, &mut offset);
        if frame != last_frame {
            tokio::select! { biased; _ = interrupt.recv() => break, result = screen.write(frame.as_bytes()) => result? }
            last_frame = frame;
        }
        if rows.iter().all(|row| row.done) {
            drop(screen);
            for row in &rows {
                print!(
                    "{}",
                    presentation::snapshot(std::slice::from_ref(&row.display), style, false)
                );
                if !row.succeeded {
                    print_action(&row.display);
                }
            }
            if rows.iter().any(|row| row.display.state == "Disconnected") {
                bail!(
                    "fleet observation lost. Accepted deployments may still run. Query status before submitting again"
                );
            }
            if rows.iter().any(|row| !row.succeeded) {
                bail!("one or more deployments did not succeed. See the target summaries above");
            }
            return Ok(());
        }
        tokio::select! {
            biased;
            _ = interrupt.recv() => break,
            input = screen.input.read() => {
                let input = input?;
                if input.is_empty() { break; }
                for action in keys.push(&input) {
                    match action {
                        Key::Detach => { drop(screen); print_detached(&rows); return Ok(()); },
                        Key::Down => offset = offset.saturating_add(1),
                        Key::Up => offset = offset.saturating_sub(1),
                        Key::Page => offset = offset.saturating_add(size.rows.saturating_sub(6).max(1)),
                        Key::Top => offset = 0,
                        Key::Bottom => offset = usize::MAX,
                    }
                }
            },
            update = updates.recv() => if let Some(Update { index, result }) = update {
                let row = &mut rows[index];
                match result {
                    Ok(job) => {
                        row.display = DisplayRow::job(&row.display.target, &job);
                        row.done = job.phase.is_terminal();
                        row.succeeded = job.phase == DeployPhase::Succeeded;
                    },
                    Err(error) => {
                        row.display = DisplayRow::error(&row.display.target, row.display.job, "Disconnected", &error.to_string());
                        row.done = true;
                    },
                }
            } else {
                for row in rows.iter_mut().filter(|row| !row.done) {
                    row.display = DisplayRow::error(&row.display.target, row.display.job, "Disconnected", "Status observer stopped.");
                    row.done = true;
                }
            },
            _ = resize.tick() => {},
        }
    }
    drop(screen);
    print_detached(&rows);
    // Dropping JoinSet aborts read-only observers, never deployment execution.
    Ok(())
}
fn print_action(row: &DisplayRow) {
    // A target is not interpolated into a shell command: arbitrary aliases may
    // contain shell syntax. Quoted commands are produced by the caller CLI docs.
    println!("  Inspect: coeus status {}", shell_word(&row.target));
    if let Some(id) = row.job {
        println!(
            "  Logs: coeus logs {id} --target {}",
            shell_word(&row.target)
        );
    }
}
pub(super) fn shell_word(text: &str) -> String {
    if !text.is_empty()
        && text
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"-_.:[]".contains(&c))
    {
        text.into()
    } else {
        format!("'{}'", text.replace('\'', "'\\''"))
    }
}
fn print_detached(rows: &[Row]) {
    println!("Detached. Accepted deployments continue.");
    for row in rows {
        if row.display.job.is_some() {
            print_action(&row.display);
        }
    }
}

#[derive(Default)]
struct Keys {
    escape: u8,
}
enum Key {
    Up,
    Down,
    Page,
    Top,
    Bottom,
    Detach,
}
impl Keys {
    fn push(&mut self, bytes: &[u8]) -> Vec<Key> {
        let mut result = Vec::new();
        for byte in bytes {
            let action = match (self.escape, byte) {
                (0, b'\x1b') => {
                    self.escape = 1;
                    None
                }
                (1, b'[') => {
                    self.escape = 2;
                    None
                }
                (2, b'A') => {
                    self.escape = 0;
                    Some(Key::Up)
                }
                (2, b'B') => {
                    self.escape = 0;
                    Some(Key::Down)
                }
                (1 | 2, _) => {
                    self.escape = 0;
                    None
                }
                (_, b'q') => Some(Key::Detach),
                (_, b'j') => Some(Key::Down),
                (_, b'k') => Some(Key::Up),
                (_, b' ') => Some(Key::Page),
                (_, b'g') => Some(Key::Top),
                (_, b'G') => Some(Key::Bottom),
                _ => None,
            };
            if let Some(action) = action {
                result.push(action);
            }
        }
        result
    }
}

fn render(rows: &[Row], size: Size, style: Style, offset: &mut usize) -> String {
    let width = size.columns.saturating_sub(1).max(1);
    let height = size.rows.max(1);
    let mut lines = Vec::new();
    lines.push(style.paint(&presentation::clip("Coeus fleet", width), Tone::Normal));
    let running = rows.iter().filter(|row| !row.done).count();
    let success = rows.iter().filter(|row| row.succeeded).count();
    let issues = rows.iter().filter(|row| row.done && !row.succeeded).count();
    lines.push(presentation::clip(
        &format!("Active {running}  OK {success}  Issues {issues}"),
        width,
    ));
    let display: Vec<_> = rows.iter().map(|row| row.display.clone()).collect();
    let body = presentation::table(&display, width, style);
    let capacity = height.saturating_sub(4);
    *offset = (*offset).min(body.len().saturating_sub(capacity.max(1)));
    let end = (*offset + capacity).min(body.len());
    lines.extend(body[*offset..end].iter().cloned());
    if body.len() > capacity {
        lines.push(presentation::clip(
            &format!(
                "Lines {}-{end}/{}  j/k scroll  g/G ends",
                *offset + 1,
                body.len()
            ),
            width,
        ));
    } else {
        lines.push(String::new());
    }
    lines.push(style.paint(
        &presentation::clip("q/Ctrl+C detach. Jobs continue.", width),
        Tone::Muted,
    ));
    lines.truncate(height);
    // No full-screen clear. Rewrite only when content or terminal size changes,
    // erase stale tails, and reserve the last column to avoid terminal wrapping.
    format!("\x1b[H{}\x1b[K\x1b[J", lines.join("\x1b[K\r\n"))
}

#[cfg(not(unix))]
pub(super) async fn follow(_targets: Vec<AcceptedTarget>, _failures: &[DisplayRow]) -> Result<()> {
    bail!("fleet terminal view requires Unix. Use --no-tty");
}

#[cfg(test)]
mod tests {
    use super::*;
    use unicode_width::UnicodeWidthStr;
    #[test]
    fn rendered_fleet_fits_every_geometry_and_all_rows_can_be_scrolled() {
        let rows: Vec<_> = (0..20)
            .map(|index| Row {
                display: DisplayRow::job(
                    &format!("machine-{index}-机器🦀"),
                    &JobStatus {
                        job_id: u64::MAX,
                        revision: "long-revision-name".into(),
                        commit: None,
                        phase: if index % 2 == 0 {
                            DeployPhase::Build
                        } else {
                            DeployPhase::Interrupted
                        },
                        cancel_requested: true,
                        message: Some("No terminal outcome.\t\x1b[2J".into()),
                    },
                ),
                done: false,
                succeeded: false,
            })
            .collect();
        for columns in [12, 32, 40, 60, 80, 120] {
            for height in [4, 8, 24, 40] {
                let mut offset = 0;
                let frame = render(
                    &rows,
                    Size {
                        columns,
                        rows: height,
                    },
                    Style { color: false },
                    &mut offset,
                );
                let mut parser = crate::utils::plain_text::PlainText::default();
                let mut visible = String::new();
                parser.push(frame.as_bytes(), |c| visible.push(c));
                assert!(visible.lines().count() <= height);
                for line in visible.lines() {
                    assert!(
                        UnicodeWidthStr::width(line.trim_end_matches('\r')) < columns,
                        "{columns}: {line:?}"
                    );
                }
                assert!(!visible.contains(['\t', '—', '–']));
                let first = frame;
                offset = usize::MAX;
                let last = render(
                    &rows,
                    Size {
                        columns,
                        rows: height,
                    },
                    Style { color: false },
                    &mut offset,
                );
                assert_ne!(first, last);
                assert!(offset > 0);
            }
        }
    }
    #[test]
    fn split_arrow_keys_and_detach_do_not_turn_into_job_input() {
        let mut keys = Keys::default();
        assert!(keys.push(b"\x1b[").is_empty());
        assert!(matches!(keys.push(b"B").as_slice(), [Key::Down]));
        assert!(matches!(
            keys.push(b"kq").as_slice(),
            [Key::Up, Key::Detach]
        ));
        assert_eq!(shell_word("host:12"), "host:12");
        assert_eq!(shell_word("host'$(bad)"), "'host'\\''$(bad)'");
    }
}
