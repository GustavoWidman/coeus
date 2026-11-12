use eyre::{Result, bail};

use crate::{common::model::Hostname, config::ClientSettings};

/// the machines a command acts on, from expressions like `altar`, `@all`,
/// `@server` or `!desktop-nixos`. names outside the inventory are tailnet
/// hostnames; groups come from the inventory's tags. exclusions alone start
/// from `@all`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Targets(Vec<Hostname>);

impl Targets {
    pub fn select(expressions: &[String], settings: &ClientSettings) -> Result<Self> {
        let mut selected: Vec<Hostname> = Vec::new();
        let only_exclusions = expressions
            .iter()
            .all(|expression| expression.starts_with('!'));

        if only_exclusions {
            selected.extend(settings.hosts.keys().cloned());
        }

        for expression in expressions {
            let (exclude, expression) = match expression.strip_prefix('!') {
                Some(rest) => (true, rest),
                None => (false, expression.as_str()),
            };

            let hosts = Self::expand(expression, settings)?;
            match exclude {
                true => selected.retain(|host| !hosts.contains(host)),
                false => {
                    for host in hosts {
                        if !selected.contains(&host) {
                            selected.push(host);
                        }
                    }
                }
            }
        }

        if selected.is_empty() {
            bail!("no machine selected");
        }

        Ok(Self(selected))
    }

    fn expand(expression: &str, settings: &ClientSettings) -> Result<Vec<Hostname>> {
        let Some(group) = expression.strip_prefix('@') else {
            return Ok(vec![expression.parse()?]);
        };

        let hosts: Vec<Hostname> = settings
            .hosts
            .iter()
            .filter(|(_, entry)| group == "all" || entry.tags.iter().any(|tag| tag == group))
            .map(|(host, _)| host.clone())
            .collect();

        if hosts.is_empty() {
            bail!("no machine in the inventory matches @{group}");
        }

        Ok(hosts)
    }

    pub fn hosts(&self) -> &[Hostname] {
        &self.0
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}
