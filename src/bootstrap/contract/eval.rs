//! one `nix eval` of the contract file, and what its failure says

use std::{fmt, path::Path, process::Stdio};

use eyre::WrapErr;
use tokio::process::Command;

use crate::nix::strip_ansi;

/// `nix eval --json --file <file> --apply <apply>`'s output. a plain file
/// needs no flake, so only `nix-command` is enabled, and nothing is asked
/// that needs a trusted user. `--show-trace` stays off: its frames are
/// nix's, not the contract's.
pub async fn json(file: &Path, apply: &str) -> eyre::Result<Vec<u8>> {
    let output = Command::new("nix")
        .args(["--extra-experimental-features", "nix-command"])
        .args(["eval", "--json", "--file"])
        .arg(file)
        .args(["--apply", apply])
        .stdin(Stdio::null())
        .kill_on_drop(true)
        .output()
        .await
        .wrap_err("failed to run nix")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(Failure::read(&strip_ansi(&stderr)).into());
    }
    Ok(output.stdout)
}

/// why an evaluation failed
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Failure {
    /// the contract refused with `throw` or `abort`: words for the person,
    /// shown as they are
    Said(String),
    /// anything else nix stopped on, with where when it said
    Broken { error: String, at: Option<String> },
}

impl Failure {
    /// from nix's stderr: its innermost `error:`, without the frames above
    /// it or the source excerpt under it
    pub fn read(stderr: &str) -> Self {
        let lines: Vec<&str> = stderr.lines().collect();
        let Some(start) = lines
            .iter()
            .rposition(|line| line.trim_start().starts_with("error:"))
        else {
            return Self::Broken {
                error: last_words(stderr),
                at: None,
            };
        };

        let first = lines[start];
        // nix indents a message's further lines under its first, past a
        // top-level `error: `
        let indent = match first.len() - first.trim_start().len() {
            0 => "error: ".len(),
            indent => indent,
        };
        let mut message = vec![first.trim_start()["error:".len()..].trim().to_owned()];
        let mut at = None;
        for line in &lines[start + 1..] {
            let deeper = line.len() - line.trim_start().len() >= indent;
            let text = line.get(indent..).unwrap_or_default();
            if let Some(position) = position(text) {
                at = Some(position.to_owned());
                break;
            }
            if !deeper || line.trim().is_empty() {
                break;
            }
            message.push(text.to_owned());
        }
        let message = message.join("\n").trim().to_owned();

        let thrown = lines[..start]
            .iter()
            .rev()
            .find(|line| line.trim_start().starts_with('…'))
            .is_some_and(|frame| frame.contains("'throw' builtin"));
        if let Some(aborted) = message
            .strip_prefix("evaluation aborted with the following error message: '")
            .and_then(|said| said.strip_suffix('\''))
        {
            return Self::Said(aborted.to_owned());
        }
        match thrown {
            true => Self::Said(message),
            false => Self::Broken { error: message, at },
        }
    }
}

/// `at /path/to/file.nix:12:5:`, nix's position line, without `at` and the
/// final colon
fn position(line: &str) -> Option<&str> {
    line.strip_prefix("at ")?.strip_suffix(':')
}

fn last_words(stderr: &str) -> String {
    stderr
        .lines()
        .map(str::trim)
        .rfind(|line| !line.is_empty())
        .unwrap_or("nix failed")
        .to_owned()
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Said(words) => f.write_str(words),
            Self::Broken { error, at: None } => write!(f, "the contract failed: {error}"),
            Self::Broken {
                error,
                at: Some(at),
            } => write!(f, "the contract failed at {at}: {error}"),
        }
    }
}

impl std::error::Error for Failure {}
