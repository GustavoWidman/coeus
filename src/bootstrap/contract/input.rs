//! what coeus tells the contract about the machine: `{ name, hostKey,
//! facts, answers }`

use serde::Serialize;

use super::Answers;
use crate::{
    bootstrap::{SshKey, remote::Facts},
    common::model::Hostname,
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Input {
    /// the machine's fleet name
    pub name: Hostname,
    /// the pinned key, without a comment
    pub host_key: SshKey,
    pub facts: Facts,
    /// questions not answered yet are absent
    pub answers: Answers,
}

impl Input {
    /// `builtins.fromJSON "<the input as JSON>"`, ready to apply a contract
    /// function to inside `--apply`, so no file or `--impure` is needed
    pub fn to_nix(&self) -> eyre::Result<String> {
        Ok(from_json(&serde_json::to_string(self)?))
    }
}

/// `text` as a Nix string literal: `"`, `\` and `${` escaped, so nothing in
/// it is interpolated
pub fn nix_string(text: &str) -> String {
    let mut literal = String::with_capacity(text.len() + 2);
    literal.push('"');
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => literal.push_str("\\\""),
            '\\' => literal.push_str("\\\\"),
            '$' if chars.peek() == Some(&'{') => literal.push_str("\\$"),
            // nix reads a raw carriage return as a newline; escaped, it stays
            '\r' => literal.push_str("\\r"),
            c => literal.push(c),
        }
    }
    literal.push('"');
    literal
}

/// `builtins.fromJSON` of a JSON document
pub(super) fn from_json(json: &str) -> String {
    format!("(builtins.fromJSON {})", nix_string(json))
}
