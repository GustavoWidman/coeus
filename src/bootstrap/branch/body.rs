//! the pull request body: the contract's text, then coeus's section between
//! `<!-- coeus:bootstrap -->` markers. a run rewrites only its section, so
//! the person may write anywhere else.

use crate::bootstrap::contract::RepoPath;

pub const START: &str = "<!-- coeus:bootstrap -->";
pub const END: &str = "<!-- /coeus:bootstrap -->";

/// what the section says, plain data the run fills from each step's result
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Evidence {
    pub host: String,
    /// origin's default branch, which merging the pull request moves
    pub base: String,
    /// `NixOS 26.05 · x86_64-linux · EFI · 16 GiB`
    pub machine: String,
    /// `ssh-ed25519 AAAA…`
    pub host_key: String,
    pub fingerprint: String,
    /// the admin that enrolled it
    pub enrolled_by: String,
    pub files: Vec<RepoPath>,
    pub created: Vec<RepoPath>,
    pub rekeyed: Vec<RepoPath>,
    /// `job #1 · generation 2 · nixos-system-nova-…`, once switched
    pub switch: Option<String>,
    /// coeusd's version and address, once it answered
    pub coeusd: Option<String>,
    /// how it booted, once known
    pub boot: Option<String>,
}

/// the section, markers included
pub fn section(evidence: &Evidence) -> String {
    let host = &evidence.host;
    let pending = |value: &Option<String>, missing: &str| match value {
        Some(value) => cell(value),
        None => format!("_{missing}_"),
    };

    let mut lines = vec![
        START.to_owned(),
        format!("### {} joins the fleet", cell(host)),
        String::new(),
        "| | |".to_owned(),
        "|---|---|".to_owned(),
        format!("| platform | {} |", cell(&evidence.machine)),
        format!("| host key | {} |", code(&evidence.fingerprint)),
        format!("| enrolled by | {} |", cell(&evidence.enrolled_by)),
        format!(
            "| first switch | {} |",
            pending(&evidence.switch, "not yet")
        ),
        format!(
            "| coeusd | {} |",
            pending(&evidence.coeusd, "not answering yet")
        ),
        format!("| boot | {} |", pending(&evidence.boot, "not checked yet")),
        String::new(),
    ];

    lines.extend(listed("files", &evidence.files));
    lines.extend(listed("secrets created", &evidence.created));

    if evidence.rekeyed.is_empty() {
        lines.push("no other secret needed rekeying".to_owned());
    } else {
        let count = evidence.rekeyed.len();
        let noun = if count == 1 { "secret" } else { "secrets" };
        lines.push("<details>".to_owned());
        lines.push(format!(
            "<summary>{count} {noun} rekeyed so {host} opens them</summary>"
        ));
        lines.push(String::new());
        lines.extend(
            evidence
                .rekeyed
                .iter()
                .map(|path| format!("- {}", code(path.as_str()))),
        );
        lines.push(String::new());
        lines.push("</details>".to_owned());
    }

    lines.push(String::new());
    lines.push(format!(
        "merging tells every machine about {host}; run `coeus bootstrap {host}` again before \
         merging if {} moved",
        code(&evidence.base)
    ));
    lines.push(END.to_owned());
    lines.join("\n")
}

/// `body` with its coeus section replaced by `section`, or appended
pub fn splice(body: &str, section: &str) -> String {
    let Some(start) = body.find(START) else {
        let before = body.trim_end();
        if before.is_empty() {
            return format!("{section}\n");
        }
        return format!("{before}\n\n{section}\n");
    };
    // a section whose end marker was deleted runs to the end of the body
    let end = body[start..]
        .find(END)
        .map_or(body.len(), |end| start + end + END.len());
    format!("{}{section}{}", &body[..start], &body[end..])
}

/// a list under a bold heading, or a line saying there is none
fn listed(heading: &str, paths: &[RepoPath]) -> Vec<String> {
    if paths.is_empty() {
        return vec![format!("**{heading}**: none"), String::new()];
    }
    let mut lines = vec![format!("**{heading}**")];
    lines.extend(
        paths
            .iter()
            .map(|path| format!("- {}", code(path.as_str()))),
    );
    lines.push(String::new());
    lines
}

/// text that stays inside one table cell
fn cell(text: &str) -> String {
    text.replace('|', r"\|").replace(['\r', '\n'], " ")
}

/// an inline code span that holds any backticks in `text`
fn code(text: &str) -> String {
    let longest = text.split(|c| c != '`').map(str::len).max().unwrap_or(0);
    let fence = "`".repeat(longest + 1);
    let pad = if text.starts_with('`') || text.ends_with('`') {
        " "
    } else {
        ""
    };
    cell(&format!("{fence}{pad}{text}{pad}{fence}"))
}
