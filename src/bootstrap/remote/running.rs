//! what the machine runs right now and which boot it runs it in, read over
//! ssh: whether a reboot into the fleet's system is still owed, and whether
//! one happened, are decided from this and never remembered

use eyre::{Result, eyre};

use crate::common::model::StorePath;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Running {
    /// `/run/current-system`'s target
    pub system: Option<StorePath>,
    /// the kernel's boot id, new on every boot
    pub boot: Option<String>,
}

impl Running {
    /// [super::Call::Running]'s `key=value` lines; a key left empty is
    /// unknown
    pub fn parse(output: &str) -> Result<Self> {
        let value = |key: &str| {
            output
                .lines()
                .find_map(|line| line.trim_end().strip_prefix(key)?.strip_prefix('='))
                .filter(|value| !value.is_empty())
        };
        let system = value("system")
            .map(|path| {
                path.parse()
                    .map_err(|error| eyre!("the machine runs {path}, not a store path: {error}"))
            })
            .transpose()?;
        Ok(Self {
            system,
            boot: value("boot").map(str::to_owned),
        })
    }
}
