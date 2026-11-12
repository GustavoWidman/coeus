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
        // TODO: serde_json::to_string(self), then nix_string()
        todo!()
    }
}

/// `text` as a Nix string literal: `"`, `\` and `${` escaped, so nothing in
/// it is interpolated
pub fn nix_string(text: &str) -> String {
    todo!()
}
