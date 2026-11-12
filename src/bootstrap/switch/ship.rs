//! what goes over the wire to a stock machine: the clone, and the inputs only
//! this admin can reach. public inputs the machine fetches itself, and only
//! those its evaluation touches: a fleet's inputs lock many copies of
//! nixpkgs that one machine never reads.

use std::collections::BTreeMap;

use eyre::{Result, eyre};
use serde::Deserialize;

use crate::common::model::StorePath;

/// `nix flake archive --json`: a flake's source and, recursively, its inputs'
#[derive(Clone, Debug, Deserialize)]
pub struct Archived {
    pub path: StorePath,
    #[serde(default)]
    pub inputs: BTreeMap<String, Archived>,
}

impl Archived {
    pub fn parse(printed: &str) -> Result<Self> {
        serde_json::from_str(printed.trim())
            .map_err(|error| eyre!("nix flake archive printed something else: {error}"))
    }
}

/// the parts of `flake.lock` that say where each input comes from
#[derive(Debug, Deserialize)]
pub struct Lock {
    nodes: BTreeMap<String, Node>,
    root: String,
}

#[derive(Debug, Deserialize)]
struct Node {
    /// each input's node key, or a `follows` path: a list, naming a node
    /// that is reached on its own
    #[serde(default)]
    inputs: BTreeMap<String, serde_json::Value>,
    locked: Option<Locked>,
}

#[derive(Debug, Deserialize)]
struct Locked {
    #[serde(rename = "type")]
    kind: String,
    url: Option<String>,
}

impl Locked {
    /// whether a machine with no credentials of the fleet can fetch it:
    /// forges and https are public, ssh and paths on this admin are not
    fn is_public(&self) -> bool {
        let remote = |url: &str| url.starts_with("https://") || url.starts_with("http://");
        match self.kind.as_str() {
            "github" | "gitlab" | "sourcehut" | "indirect" => true,
            "git" | "tarball" | "file" | "mercurial" => self.url.as_deref().is_some_and(remote),
            _ => false,
        }
    }
}

impl Lock {
    pub fn parse(text: &str) -> Result<Self> {
        serde_json::from_str(text).map_err(|error| eyre!("flake.lock does not parse: {error}"))
    }

    /// the clone's source, then every input the machine could not fetch
    /// itself, each once
    pub fn private_paths(&self, archived: &Archived) -> Vec<StorePath> {
        let mut paths = vec![archived.path.clone()];
        let mut stack = vec![(self.root.as_str(), archived)];
        while let Some((key, archived)) = stack.pop() {
            let Some(node) = self.nodes.get(key) else {
                continue;
            };
            for (name, reference) in &node.inputs {
                let (Some(child), Some(input)) = (reference.as_str(), archived.inputs.get(name))
                else {
                    continue;
                };
                let public = self
                    .nodes
                    .get(child)
                    .and_then(|node| node.locked.as_ref())
                    .is_some_and(Locked::is_public);
                if !public && !paths.contains(&input.path) {
                    paths.push(input.path.clone());
                }
                stack.push((child, input));
            }
        }
        paths
    }
}
