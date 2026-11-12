use colored::Colorize;

use super::{format, table::Table};
use crate::common::model::{ClosureDiff, PackageChange};

/// a closure diff as a table of packages, at most `limit` of them, and a
/// line of totals
pub fn lines(diff: &ClosureDiff, limit: Option<usize>) -> Vec<String> {
    if diff.is_empty() {
        return vec!["no package changes".dimmed().to_string()];
    }

    let shown = limit.unwrap_or(usize::MAX).min(diff.changes.len());
    let mut table = Table::new(&["", "PACKAGE", "VERSION", "SIZE"]);
    for change in &diff.changes[..shown] {
        table.row(row(change));
    }

    let mut lines = table.lines();
    if shown < diff.changes.len() {
        lines.push(
            format!("… {} more", diff.changes.len() - shown)
                .dimmed()
                .to_string(),
        );
    }
    lines.push(totals(diff));
    lines
}

/// `3 added · 1 removed · 12 changed · +34.2 MiB`
pub fn totals(diff: &ClosureDiff) -> String {
    let added = diff
        .changes
        .iter()
        .filter(|change| change.is_added())
        .count();
    let removed = diff
        .changes
        .iter()
        .filter(|change| change.is_removed())
        .count();
    let changed = diff.changes.len() - added - removed;

    let counts = [
        (added, "added".green()),
        (removed, "removed".red()),
        (changed, "changed".yellow()),
    ]
    .into_iter()
    .filter(|(count, _)| *count > 0)
    .map(|(count, label)| format!("{} {label}", count.to_string().bold()));

    counts
        .chain(std::iter::once(size(diff.size_delta())))
        .collect::<Vec<_>>()
        .join(" · ")
}

fn row(change: &PackageChange) -> Vec<String> {
    let versions = |versions: &[String]| match versions.is_empty() {
        true => "∅".to_owned(),
        false => versions.join(", "),
    };

    let (mark, version) = match (change.is_added(), change.is_removed()) {
        (true, _) => (
            "+".green().bold(),
            versions(&change.after).green().to_string(),
        ),
        (_, true) => ("-".red().bold(), versions(&change.before).red().to_string()),
        _ if change.before.is_empty() => ("~".yellow().bold(), String::new()),
        _ => (
            "~".yellow().bold(),
            format!(
                "{} {} {}",
                versions(&change.before).dimmed(),
                "→".dimmed(),
                versions(&change.after).yellow()
            ),
        ),
    };

    vec![
        mark.to_string(),
        change.name.bold().to_string(),
        version,
        size(change.size_delta),
    ]
}

fn size(delta: i64) -> String {
    let text = format::size_delta(delta);
    match delta {
        0 => text.dimmed().to_string(),
        1.. => text.yellow().to_string(),
        _ => text.green().to_string(),
    }
}
