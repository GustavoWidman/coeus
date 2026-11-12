use colored::Colorize;
use ratatui::{
    Frame,
    layout::Rect,
    widgets::{Padding, Paragraph},
};

use super::{pane, quiet, text};
use crate::{
    client::{Checkup, Finding, Link, Severity},
    common::model::{JobSummary, JobView, MachineView, Phase, Timestamp},
    ui::{
        Theme,
        board::{Row, eta},
        format, history,
        top::Top,
    },
};

pub fn draw(frame: &mut Frame<'_>, area: Rect, top: &Top, now: Timestamp) {
    let Some(row) = top.selected() else {
        frame.render_widget(pane("machine".bold().to_string()), area);
        return;
    };

    let block = pane(Theme::host(row.host.as_str()).to_string()).padding(Padding::horizontal(1));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let lines = lines(row, top.version, now);
    frame.render_widget(Paragraph::new(text(&lines, inner.width)), inner);
}

/// the selected machine, most urgent first: what it runs, what it does,
/// what is wrong with it, and what it did
pub fn lines(row: &Row, version: &str, now: Timestamp) -> Vec<String> {
    let Some(view) = &row.view else {
        return unconnected(row);
    };

    let mut lines = vec![identity(view, &row.link)];
    lines.extend(system(view));
    lines.push(String::new());

    match &view.running {
        Some(job) => lines.extend(running(view, job, now)),
        None => lines.push(quiet("nothing running")),
    }
    if let Some(next) = &view.next {
        lines.push(waiting(next, now));
    }

    lines.push(String::new());
    lines.push(heading("doctor"));
    lines.extend(findings(&Checkup::of(view, version).findings));

    if !view.recent.is_empty() {
        lines.push(String::new());
        lines.push(heading("recent jobs"));
        let recent: Vec<_> = view
            .recent
            .iter()
            .map(|job| (row.host.clone(), job.clone()))
            .collect();
        lines.extend(history::jobs(&recent, now));
    }

    lines
}

fn unconnected(row: &Row) -> Vec<String> {
    match &row.link {
        Link::Unreachable(reason) => {
            findings(&Checkup::unreachable(row.host.clone(), reason).findings)
        }
        _ => vec![quiet(format!("connecting to {}…", row.host))],
    }
}

/// `nixos · x86_64-linux · coeusd 0.1.0   ● connected`
fn identity(view: &MachineView, link: &Link) -> String {
    let host = &view.host;
    let link = match link {
        Link::Connected => format!("{} connected", "●".green()),
        Link::Connecting => format!("{} reconnecting", "↻".yellow().bold()),
        Link::Unreachable(reason) => format!("{} {}", "⊘ unreachable:".red(), reason.dimmed()),
    };
    format!(
        "{}   {link}",
        format!(
            "{} · {} · coeusd {}",
            host.platform, host.system, host.version
        )
        .dimmed()
    )
}

/// what runs, and the ref the machine catches up with
fn system(view: &MachineView) -> Vec<String> {
    let system = &view.system;
    let deployed = match &system.deployed {
        Some(source) => source.to_string().cyan().bold().to_string(),
        None => "a system coeus did not deploy".dimmed().to_string(),
    };
    let generation = system
        .generation
        .map(|number| format!("  generation {}", number.to_string().bold()))
        .unwrap_or_default();

    let mut lines = vec![format!("{} {deployed}{generation}", label("runs"))];
    if let Some(current) = &system.current {
        lines.push(format!("{} {}", label(""), current.name().dimmed()));
    }

    if let Some(follow) = &system.follow {
        let latest = follow
            .latest
            .as_ref()
            .map(|latest| format!(" {} {}", "→".dimmed(), latest.rev.short().cyan()))
            .unwrap_or_default();
        let paused = match follow.paused {
            true => format!("  {}", "holding until it moves".yellow()),
            false => String::new(),
        };
        lines.push(format!(
            "{} {}{latest}{paused}",
            label("follows"),
            follow.git_ref.bold()
        ));
    }
    lines
}

fn running(view: &MachineView, job: &JobView, now: Timestamp) -> Vec<String> {
    let summary = &job.summary;
    let mut lines = vec![
        format!(
            "{} {}  {}",
            Theme::spinner(now),
            format!("job {}", summary.id).bold(),
            provenance(summary)
        ),
        format!("  {}", timeline(job, now)),
    ];

    let elapsed = summary
        .started_at
        .map(|started| format::duration(started.until(now)))
        .unwrap_or_default();
    let left = eta(view, job, now)
        .map(|left| {
            format!("  ~{} left", format::duration(left))
                .dimmed()
                .to_string()
        })
        .unwrap_or_default();
    let gauge = match job.progress.fraction() {
        Some(fraction) => format!(
            "{} {}  ",
            Theme::bar(fraction, 24),
            format::percent(fraction).bold()
        ),
        None => String::new(),
    };
    lines.push(format!("  {gauge}{} {elapsed}{left}", "running".dimmed()));

    let counts = format::counts(&job.progress);
    if !counts.is_empty() {
        lines.push(format!("  {counts}"));
    }
    lines.extend(
        job.progress
            .active
            .iter()
            .map(|name| format!("  {} {}", "⚙".yellow(), format::derivation(name))),
    );
    if let Some(changes) = job.changes {
        lines.push(format!(
            "  {}",
            format!("{changes} packages change").dimmed()
        ));
    }
    lines
}

/// `✓ fetching › ✓ evaluating › building 1m02s › activating`
fn timeline(job: &JobView, now: Timestamp) -> String {
    let current = job.summary.state.phase().unwrap_or(Phase::Preparing);
    let summary = &job.summary;

    let mut stages = Vec::new();
    if summary.source.builds() {
        stages.extend([Phase::Fetching, Phase::Evaluating, Phase::Building]);
    }
    if summary.action.activates() || summary.action.sets_profile() {
        stages.push(Phase::Activating);
    }
    if summary.action.activates() {
        stages.push(Phase::Verifying);
    }

    stages
        .into_iter()
        .map(|stage| match stage.cmp(&current) {
            std::cmp::Ordering::Less => format!("{} {}", "✓".green(), stage.as_str().dimmed()),
            std::cmp::Ordering::Equal => {
                let took = job
                    .phase_started_at
                    .map(|started| format!(" {}", format::duration(started.until(now))))
                    .unwrap_or_default();
                format!("{}{}", Theme::phase(stage).bold(), took.dimmed())
            }
            std::cmp::Ordering::Greater => stage.as_str().bright_black().to_string(),
        })
        .collect::<Vec<_>>()
        .join(&format!(" {} ", "›".dimmed()))
}

fn waiting(next: &JobSummary, now: Timestamp) -> String {
    format!(
        "{} {}  {}  {}",
        "○".white(),
        format!("then job {}", next.id).bold(),
        provenance(next),
        format!("queued {}", format::ago(next.ordered_at, now)).dimmed()
    )
}

/// `build main@caf4e2b by r3dlust@altar`
fn provenance(summary: &JobSummary) -> String {
    format!(
        "{} {} {}",
        summary.action.as_str().bold(),
        summary.source.to_string().cyan(),
        format!("by {}", summary.origin).dimmed()
    )
}

fn findings(findings: &[Finding]) -> Vec<String> {
    findings
        .iter()
        .flat_map(|finding| {
            let (glyph, title) = match finding.severity {
                Severity::Ok => ("✓".green().bold(), finding.title.normal()),
                Severity::Warn => ("!".yellow().bold(), finding.title.yellow()),
                Severity::Fail => ("✗".red().bold(), finding.title.red()),
            };
            let fix = finding
                .fix
                .as_ref()
                .map(|fix| format!("    {} {}", "→".dimmed(), fix.dimmed()));
            std::iter::once(format!("{glyph} {title}")).chain(fix)
        })
        .collect()
}

fn heading(title: &str) -> String {
    title.bold().to_string()
}

fn label(text: &str) -> String {
    format!("{text:<8}").dimmed().to_string()
}
