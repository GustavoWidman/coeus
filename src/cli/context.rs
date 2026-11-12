use std::io::IsTerminal;

use eyre::Result;

use crate::{
    client::{Fleet, Targets},
    common::model::Hostname,
    config::ClientSettings,
};

/// what every command starts from: the settings, and how output looks
pub struct Context {
    pub settings: ClientSettings,
    /// stdout is a terminal that can redraw
    pub live: bool,
}

impl Context {
    pub fn new(settings: ClientSettings) -> Self {
        Self {
            settings,
            live: std::io::stdout().is_terminal(),
        }
    }

    /// the machines `expressions` select, or every machine of the inventory
    /// (this one when the inventory is empty) when there are none
    pub fn targets_or_all(&self, expressions: &[String]) -> Result<Targets> {
        match expressions.is_empty() {
            true if self.settings.hosts.is_empty() => self.targets(&[local_hostname()]),
            true => self.targets(&["@all".to_owned()]),
            false => self.targets(expressions),
        }
    }

    pub fn targets(&self, expressions: &[String]) -> Result<Targets> {
        Targets::select(expressions, &self.settings)
    }

    pub fn fleet(&self, targets: Targets) -> Result<Fleet> {
        let key = self.settings.key.load()?;
        Ok(Fleet::new(self.settings.clone(), key, targets))
    }

    /// a fleet of one machine
    pub fn machine(&self, host: &Hostname) -> Result<Fleet> {
        self.fleet(self.targets(&[host.to_string()])?)
    }
}

/// this machine's name without its domain
pub fn local_hostname() -> String {
    rustix::system::uname()
        .nodename()
        .to_string_lossy()
        .split('.')
        .next()
        .unwrap_or_default()
        .to_owned()
}
