use std::{
    collections::BTreeMap,
    ffi::OsString,
    path::{Path, PathBuf},
};

use eyre::{Result, eyre};
use serde::Deserialize;
use tokio::process::Command;

use crate::{
    common::model::{NixOptions, StorePath},
    job::JobSpec,
};

/// the caches `clean_substituters` trusts instead of whatever an older
/// generation left in `nix.conf`
pub const CLEAN_SUBSTITUTERS: &str = "https://cache.nixos.org https://nix-community.cachix.org https://r3dlust.cachix.org https://install.determinate.systems";
pub const CLEAN_TRUSTED_PUBLIC_KEYS: &str = "cache.flakehub.com-3:hJuILl5sVK4iKm86JzgdXW12Y2Hwd5G07qKtHTOcDCM= cache.nixos.org-1:6NCHdD59X431o0gWypbMrAURkbJ16ZPMQFGspcDShjY= nix-community.cachix.org-1:mB9FSh9qf2dCimDSUo8Zy7bkq5CX+/rkCWyvRCYg3Fs= r3dlust.cachix.org-1:/R3S8pW/nr7kOBJKcGPsZ0zCepvldTUEgbrqa4O3cW0=";

/// one nix call a runner makes
#[derive(Clone, Copy, Debug)]
pub enum Call<'a> {
    /// fetches a flake
    Metadata(&'a str),
    /// evaluates an installable without building it, naming its derivation
    /// and output
    Evaluate(&'a str),
    /// builds a derivation, linked from `link` so the store keeps it
    Build { drv: &'a StorePath, link: &'a Path },
    /// what changes, package by package, from one system to another
    DiffClosures {
        from: &'a StorePath,
        to: &'a StorePath,
    },
    /// a store path's metadata, its deriver among it
    PathInfo(&'a StorePath),
    /// a flake's `nixConfig`, read from its own `flake.nix`
    FlakeConfig(&'a str),
}

impl Call<'_> {
    fn args(&self) -> Vec<OsString> {
        match *self {
            Self::Metadata(flake) => words(["flake", "metadata", "--json", flake]),
            Self::Evaluate(installable) => words(["build", "--dry-run", "--json", installable]),
            Self::Build { drv, link } => vec![
                "build".into(),
                format!("{drv}^*").into(),
                "--out-link".into(),
                link.into(),
                "--print-out-paths".into(),
            ],
            Self::DiffClosures { from, to } => {
                words(["store", "diff-closures", from.as_str(), to.as_str()])
            }
            Self::PathInfo(path) => words(["path-info", "--json", path.as_str()]),
            Self::FlakeConfig(flake) => {
                let flake = flake
                    .replace('\\', "\\\\")
                    .replace('"', "\\\"")
                    .replace("${", "\\${");
                let expr = format!(
                    "let flake = builtins.getFlake \"{flake}\"; \
                     in (import (flake.outPath + \"/flake.nix\")).nixConfig or {{ }}"
                );
                vec![
                    "eval".into(),
                    "--impure".into(),
                    "--json".into(),
                    "--expr".into(),
                    expr.into(),
                ]
            }
        }
    }

    /// whether the call reads a flake, and so would update its lock file
    const fn reads_flake(&self) -> bool {
        matches!(self, Self::Metadata(_) | Self::Evaluate(_))
    }
}

/// how a runner calls nix. every call logs internal-json, which the job log
/// keeps for `nom` and the runner folds into progress, and none writes a lock
/// file into the flake.
#[derive(Clone, Debug)]
pub struct Nix {
    options: NixOptions,
    ssh_key: Option<PathBuf>,
    /// the flake's own caches, which `accept-flake-config` applies where a
    /// call reads the flake but not to a build of the derivation it named
    caches: Vec<(&'static str, String)>,
}

impl Nix {
    pub fn new(spec: &JobSpec) -> Self {
        Self {
            options: spec.order.options,
            ssh_key: spec.ssh_key_file.clone(),
            caches: Vec::new(),
        }
    }

    /// the caches and keys the flake's `nixConfig` names, from what
    /// [Call::FlakeConfig] printed. a machine that has them in `nix.conf`
    /// already loses nothing; a stock one builds what the fleet's caches hold
    pub fn use_flake_caches(&mut self, config: &[u8]) {
        let Ok(serde_json::Value::Object(config)) = serde_json::from_slice(config) else {
            return;
        };
        let joined = |names: [&str; 2]| {
            names
                .iter()
                .filter_map(|name| config.get(*name))
                .flat_map(|value| match value {
                    serde_json::Value::String(text) => {
                        text.split_whitespace().map(str::to_owned).collect()
                    }
                    serde_json::Value::Array(items) => items
                        .iter()
                        .filter_map(|item| item.as_str().map(str::to_owned))
                        .collect(),
                    _ => Vec::new(),
                })
                .collect::<Vec<_>>()
                .join(" ")
        };
        self.caches = [
            (
                "extra-substituters",
                joined(["substituters", "extra-substituters"]),
            ),
            (
                "extra-trusted-public-keys",
                joined(["trusted-public-keys", "extra-trusted-public-keys"]),
            ),
        ]
        .into_iter()
        .filter(|(_, value)| !value.is_empty())
        .collect();
    }

    pub fn args(&self, call: &Call<'_>) -> Vec<OsString> {
        let mut args = call.args();
        args.extend(words([
            "--log-format",
            "internal-json",
            "-v",
            "--extra-experimental-features",
            crate::nix::FLAKE_FEATURES,
            "--option",
            "accept-flake-config",
            "true",
        ]));

        if call.reads_flake() {
            args.push("--no-write-lock-file".into());
        }
        if self.options.no_eval_cache {
            args.extend(words(["--option", "eval-cache", "false"]));
        }
        if matches!(call, Call::Build { .. }) && !self.options.clean_substituters {
            for (name, value) in &self.caches {
                args.extend(["--option".into(), (*name).into(), value.into()]);
            }
        }
        if self.options.clean_substituters {
            let options = [
                ("substituters", CLEAN_SUBSTITUTERS),
                ("extra-substituters", ""),
                ("trusted-public-keys", CLEAN_TRUSTED_PUBLIC_KEYS),
                ("extra-trusted-public-keys", ""),
            ];
            for (name, value) in options {
                args.extend(words(["--option", name, value]));
            }
        }

        args
    }

    pub fn command(&self, call: &Call<'_>) -> Command {
        let mut command = Command::new("nix");
        command.args(self.args(call)).env("NIX_REMOTE", "daemon");

        if let Some(key) = &self.ssh_key {
            command.env(
                "GIT_SSH_COMMAND",
                format!(
                    "ssh -i {} -o IdentitiesOnly=yes -o StrictHostKeyChecking=accept-new",
                    key.display()
                ),
            );
        }

        command
    }
}

fn words<const N: usize>(words: [&str; N]) -> Vec<OsString> {
    words.into_iter().map(OsString::from).collect()
}

/// the derivation and output that `nix build --dry-run --json` names
pub fn evaluation(stdout: &[u8]) -> Result<(StorePath, StorePath)> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Installable {
        drv_path: StorePath,
        outputs: BTreeMap<String, StorePath>,
    }

    let installables: Vec<Installable> = serde_json::from_slice(stdout)?;
    let mut installable = installables
        .into_iter()
        .next()
        .ok_or_else(|| eyre!("nix evaluated nothing"))?;
    let out = installable
        .outputs
        .remove("out")
        .ok_or_else(|| eyre!("{} has no `out` output", installable.drv_path))?;

    Ok((installable.drv_path, out))
}

/// the deriver `nix path-info --json` reports, whether it keys its answer by
/// path or lists it
pub fn deriver(stdout: &[u8]) -> Option<StorePath> {
    let info: serde_json::Value = serde_json::from_slice(stdout).ok()?;
    let entry = match &info {
        serde_json::Value::Object(paths) => paths.values().next()?,
        serde_json::Value::Array(paths) => paths.first()?,
        _ => return None,
    };

    entry.get("deriver")?.as_str()?.parse().ok()
}
