//! Shared display policy for structured client output, never raw child output.
use crate::{common::proto::*, utils::plain_text};
use std::io::IsTerminal;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Tone {
    Normal,
    Active,
    Good,
    Warning,
    Bad,
    Muted,
}

#[derive(Clone, Copy)]
pub(super) struct Style {
    pub color: bool,
}
impl Style {
    pub fn stdout() -> Self {
        Self::for_output(std::io::stdout().is_terminal())
    }
    pub fn for_output(terminal: bool) -> Self {
        Self {
            color: crate::utils::log::Logger::use_color(terminal),
        }
    }
    pub fn paint(self, text: &str, tone: Tone) -> String {
        let code = match tone {
            Tone::Normal => return text.into(),
            Tone::Active => "36",
            Tone::Good => "32",
            Tone::Warning => "33",
            Tone::Bad => "31",
            Tone::Muted => "2",
        };
        if self.color {
            format!("\x1b[{code}m{text}\x1b[0m")
        } else {
            text.into()
        }
    }
}
pub(super) fn terminal_capable() -> bool {
    std::env::var("TERM").as_deref() != Ok("dumb")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Size {
    pub columns: usize,
    pub rows: usize,
}
impl Size {
    pub fn stdout() -> Self {
        #[cfg(unix)]
        if let Ok(size) = rustix::termios::tcgetwinsize(std::io::stdout())
            && size.ws_col > 0
            && size.ws_row > 0
        {
            return Self {
                columns: usize::from(size.ws_col),
                rows: usize::from(size.ws_row),
            };
        }
        Self {
            columns: 80,
            rows: 24,
        }
    }
}

pub(super) fn safe(text: &str) -> String {
    plain_text::bounded(text, 4096).replace('\t', " ")
}
/// A conservative character-cell bound also covers combining marks and wide text.
/// Summing character widths can overestimate joined emoji, never overflow a cell.
pub(super) fn clip(text: &str, width: usize) -> String {
    let text = safe(text);
    if text.chars().map(|c| c.width().unwrap_or(0)).sum::<usize>() <= width {
        return text;
    }
    if width == 0 {
        return String::new();
    }
    let mut result = String::new();
    let mut used = 0;
    for c in text.chars() {
        let cells = c.width().unwrap_or(0);
        if used + cells > width - 1 {
            break;
        }
        result.push(c);
        used += cells;
    }
    result.push('~');
    result
}
fn cell(text: &str, width: usize) -> String {
    let clipped = clip(text, width);
    let padding = width.saturating_sub(UnicodeWidthStr::width(clipped.as_str()));
    format!("{clipped}{}", " ".repeat(padding))
}

pub(super) fn phase(phase: DeployPhase) -> (&'static str, Tone) {
    match phase {
        DeployPhase::Queued => ("Queued", Tone::Muted),
        DeployPhase::Checkout => ("Checkout", Tone::Active),
        DeployPhase::Build => ("Building", Tone::Active),
        DeployPhase::Switch => ("Activating", Tone::Active),
        DeployPhase::Succeeded => ("Succeeded", Tone::Good),
        DeployPhase::Failed => ("Failed", Tone::Bad),
        DeployPhase::Cancelled => ("Cancelled", Tone::Warning),
        DeployPhase::Interrupted => ("Recovery required", Tone::Bad),
        DeployPhase::RequiresManualSwitch => ("Waiting for switch", Tone::Warning),
    }
}

#[derive(Clone, Debug)]
pub(super) struct DisplayRow {
    pub target: String,
    pub job: Option<JobId>,
    pub revision: String,
    pub state: String,
    pub tone: Tone,
    pub detail: String,
}
impl DisplayRow {
    pub fn job(target: &str, job: &JobStatus) -> Self {
        let (state, tone) = phase(job.phase);
        let mut detail = Vec::new();
        if job.cancel_requested
            && !matches!(
                job.phase,
                DeployPhase::Cancelled | DeployPhase::Succeeded | DeployPhase::Failed
            )
        {
            detail.push("Cancel requested.".to_string());
        }
        if matches!(
            job.phase,
            DeployPhase::Interrupted | DeployPhase::RequiresManualSwitch
        ) {
            detail.push("New deployments blocked.".to_string());
        }
        if let Some(message) = &job.message {
            detail.push(safe(message));
        }
        Self {
            target: safe(target),
            job: Some(job.job_id),
            revision: safe(&job.revision),
            state: state.into(),
            tone,
            detail: detail.join(" "),
        }
    }
    pub fn pending(target: &str, id: JobId) -> Self {
        Self {
            target: safe(target),
            job: Some(id),
            revision: String::new(),
            state: "Accepted".into(),
            tone: Tone::Active,
            detail: "Waiting for status.".into(),
        }
    }
    pub fn error(target: &str, id: Option<JobId>, state: &str, error: &str) -> Self {
        Self {
            target: safe(target),
            job: id,
            revision: String::new(),
            state: state.into(),
            tone: Tone::Bad,
            detail: safe(error),
        }
    }
    pub fn idle(target: &str) -> Self {
        Self {
            target: safe(target),
            job: None,
            revision: String::new(),
            state: "Idle".into(),
            tone: Tone::Muted,
            detail: "No retained jobs.".into(),
        }
    }
}

/// Shared table/card layout. No tabs, embedded remote controls, or guessed percentages.
/// Full pipe output is not clipped. Interactive cells reserve the terminal's last
/// column to avoid automatic wrapping, including on very small terminals.
pub(super) fn table(rows: &[DisplayRow], width: usize, style: Style) -> Vec<String> {
    let width = width.max(1);
    if width < 72 {
        let mut lines = Vec::new();
        for row in rows {
            lines.push(clip(&row.target, width));
            if let Some(id) = row.job {
                lines.push(clip(&format!("  Job {id}"), width));
            }
            lines.push(style.paint(&clip(&format!("  {}", row.state), width), row.tone));
            if !row.revision.is_empty() {
                lines.push(style.paint(
                    &clip(&format!("  rev {}", row.revision), width),
                    Tone::Muted,
                ));
            }
            if !row.detail.is_empty() {
                lines.push(clip(&format!("  {}", row.detail), width));
            }
        }
        return lines;
    }
    let job_width = rows
        .iter()
        .filter_map(|r| r.job)
        .map(|id| id.to_string().len())
        .max()
        .unwrap_or(3)
        .max(3);
    let target_width = (width / 4).clamp(12, 32);
    let state_width = 18;
    let rev_width = width.saturating_sub(target_width + job_width + state_width + 6);
    let mut lines = vec![style.paint(
        &format!(
            "{}  {}  {}  {}",
            cell("TARGET", target_width),
            cell("JOB", job_width),
            cell("STATE", state_width),
            cell("REVISION", rev_width)
        ),
        Tone::Muted,
    )];
    for row in rows {
        lines.push(format!(
            "{}  {}  {}  {}",
            cell(&row.target, target_width),
            cell(&row.job.map_or("-".into(), |id| id.to_string()), job_width),
            style.paint(&cell(&row.state, state_width), row.tone),
            cell(&row.revision, rev_width)
        ));
        if !row.detail.is_empty() {
            lines.push(clip(&format!("  {}", row.detail), width));
        }
    }
    lines
}

pub(super) fn snapshot(rows: &[DisplayRow], style: Style, terminal: bool) -> String {
    if terminal {
        table(rows, Size::stdout().columns.saturating_sub(1), style).join("\n") + "\n"
    } else {
        // Full identifiers and causes stay available when piped, without control codes.
        rows.iter()
            .map(|row| {
                let id = row.job.map_or(String::new(), |id| format!(" job {id}"));
                let rev = if row.revision.is_empty() {
                    String::new()
                } else {
                    format!(" (rev {})", row.revision)
                };
                let detail = if row.detail.is_empty() {
                    String::new()
                } else {
                    format!("\n  {}", row.detail)
                };
                format!(
                    "{}{id}: {}{rev}{detail}\n",
                    row.target,
                    style.paint(&row.state, row.tone)
                )
            })
            .collect()
    }
}

pub(super) fn job_details(jobs: &[JobStatus], style: Style) -> String {
    let mut output = String::new();
    for job in jobs {
        let row = DisplayRow::job("", job);
        output.push_str(&format!(
            "Job {}: {}\n  Revision: {}\n",
            job.job_id,
            style.paint(&row.state, row.tone),
            row.revision
        ));
        if !row.detail.is_empty() {
            output.push_str(&format!("  {}\n", row.detail));
        }
    }
    if jobs.is_empty() {
        output.push_str("No retained jobs.\n");
    }
    output
}

/// Keep one registered receiver across request/render/sleep boundaries. Recreating
/// ctrl_c futures leaves small gaps where SIGINT can be consumed with no waiter.
pub(super) struct Interrupt {
    #[cfg(unix)]
    signal: tokio::signal::unix::Signal,
}
impl Interrupt {
    pub fn new() -> eyre::Result<Self> {
        Ok(Self {
            #[cfg(unix)]
            signal: tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?,
        })
    }
    pub async fn recv(&mut self) -> eyre::Result<()> {
        #[cfg(unix)]
        {
            self.signal
                .recv()
                .await
                .ok_or_else(|| eyre::eyre!("interrupt listener closed"))
        }
        #[cfg(not(unix))]
        {
            tokio::signal::ctrl_c().await?;
            Ok(())
        }
    }
}

/// Changed-only snapshots in pipes; a stable alternate-screen view on a TTY.
/// The guard owns no deployment and restores the screen even on an I/O error.
pub(super) struct Watch {
    enabled: bool,
    last: String,
    #[cfg(unix)]
    output: Option<super::terminal_output::TerminalOutput>,
    entered: bool,
}
impl Watch {
    pub fn new(enabled: bool) -> eyre::Result<Self> {
        Ok(Self {
            enabled,
            last: String::new(),
            #[cfg(unix)]
            output: if enabled && std::io::stdout().is_terminal() && terminal_capable() {
                Some(super::terminal_output::TerminalOutput::new()?)
            } else {
                None
            },
            entered: false,
        })
    }
    pub async fn emit(&mut self, text: &str) -> eyre::Result<()> {
        #[cfg(unix)]
        if let Some(output) = &self.output {
            let size = Size::stdout();
            // Content already owns semantic styles. Bound each colored line by
            // stripping styles only when it needs clipping, never split an SGR.
            let width = size.columns.saturating_sub(1).max(1);
            let capacity = size.rows.saturating_sub(2);
            let source: Vec<_> = text.lines().collect();
            let mut lines: Vec<_> = source
                .iter()
                .take(capacity)
                .map(|line| {
                    if UnicodeWidthStr::width(safe(line).as_str()) > width {
                        clip(line, width)
                    } else {
                        (*line).to_string()
                    }
                })
                .collect();
            if source.len() > capacity {
                lines.push(clip("More rows: run status without --watch.", width));
            }
            lines.push(clip("Ctrl+C stops watching.", width));
            lines.truncate(size.rows.max(1));
            let frame = format!("\x1b[H{}\x1b[K\x1b[J", lines.join("\x1b[K\r\n"));
            if frame != self.last {
                if !self.entered {
                    self.entered = true;
                    output.write(b"\x1b[?1049h\x1b[?25l").await?;
                }
                output.write(frame.as_bytes()).await?;
                self.last = frame;
            }
            return Ok(());
        }
        if !self.enabled || text != self.last {
            use std::io::Write;
            let mut out = std::io::stdout().lock();
            out.write_all(text.as_bytes())?;
            out.flush()?;
            self.last = text.into();
        }
        Ok(())
    }
}
impl Drop for Watch {
    fn drop(&mut self) {
        #[cfg(unix)]
        if self.entered
            && let Some(output) = &self.output
        {
            output.cleanup(b"\x1b[0m\x1b[?25h\x1b[?1049l");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn actual_layout_fits_narrow_normal_wide_and_strips_controls() {
        let row = DisplayRow::job(
            "long\t\x1b[31m机器🦀e\u{301}alias\x1b[0m",
            &JobStatus {
                job_id: u64::MAX,
                revision: "revision-with-a-long-name".into(),
                commit: None,
                phase: DeployPhase::Interrupted,
                cancel_requested: true,
                message: Some("uncertain\r\n\x1b[2Jresult".into()),
            },
        );
        for width in [1, 12, 31, 39, 59, 71, 79, 119] {
            for line in table(std::slice::from_ref(&row), width, Style { color: false }) {
                assert!(
                    UnicodeWidthStr::width(line.as_str()) <= width,
                    "{width}: {line:?}"
                );
                assert!(!line.contains(['\t', '\r', '\n', '\x1b']));
            }
        }
        let full = snapshot(&[row], Style { color: false }, false);
        assert!(full.contains(&u64::MAX.to_string()));
        assert!(full.contains("Recovery required"));
        assert!(full.contains("New deployments blocked."));
    }
    #[test]
    fn every_phase_has_truthful_text_with_semantic_color_and_plain_equivalent() {
        for state in [
            DeployPhase::Queued,
            DeployPhase::Checkout,
            DeployPhase::Build,
            DeployPhase::Switch,
            DeployPhase::Succeeded,
            DeployPhase::Failed,
            DeployPhase::Cancelled,
            DeployPhase::Interrupted,
            DeployPhase::RequiresManualSwitch,
        ] {
            let (label, tone) = phase(state);
            let ansi = Style { color: true }.paint(label, tone);
            assert_eq!(safe(&ansi), label);
            assert!(!label.contains(['—', '–']));
            assert_eq!(Style { color: false }.paint(label, tone), label);
            assert!(ansi.starts_with("\x1b["));
        }
    }
}
