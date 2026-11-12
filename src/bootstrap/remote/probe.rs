//! what probe.sh found, parsed into types at the boundary, so a probe that
//! changes shape fails here and not in a contract template

use std::collections::BTreeMap;

use eyre::{Result, bail, eyre};

use super::{BootLoader, Facts, NixState, Platform, System};
use crate::bootstrap::SshKey;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Probed {
    pub facts: Facts,
    /// what sshd serves from `/etc/ssh/ssh_host_ed25519_key.pub`, which must
    /// be the pinned key
    pub host_key: SshKey,
    /// nix on a Mac; always installed on NixOS
    pub nix: NixState,
}

impl Probed {
    /// probe.sh's `key=value` lines, plus the hardware configuration read as
    /// root on NixOS
    pub fn parse(output: &str, hardware_config: Option<String>) -> Result<Self> {
        let lines = Lines::of(output);
        let platform = lines.platform()?;
        let system = lines.system(platform)?;
        let login_user = lines.required("loginUser")?.to_owned();
        let nix_version = lines.get("nixVersion").map(version_of);

        let nix = match (&nix_version, lines.get("nixReceipt")) {
            (Some(version), _) => NixState::Installed {
                version: version.clone(),
            },
            (None, Some(_)) => NixState::Broken,
            (None, None) => NixState::Missing,
        };
        if platform == Platform::Nixos && nix_version.is_none() {
            bail!("the machine runs NixOS, but nix --version does not answer there");
        }

        let main_user = match platform {
            Platform::Darwin => Some(
                lines
                    .get("consoleUser")
                    .filter(|user| *user != "root")
                    .unwrap_or(&login_user)
                    .to_owned(),
            ),
            Platform::Nixos => Some(login_user.clone()).filter(|user| user != "root"),
        };
        let nixos = platform == Platform::Nixos;

        let facts = Facts {
            platform,
            system,
            os_version: lines.required("osVersion")?.to_owned(),
            state_version: match nixos {
                true => Some(lines.required("stateVersion")?.to_owned()),
                false => None,
            },
            hostname: lines
                .required("hostname")?
                .split('.')
                .next()
                .unwrap_or_default()
                .to_owned(),
            efi: lines.get("efi").is_some(),
            boot_loader: match lines.get("bootLoader") {
                None => None,
                Some(loader) => Some(boot_loader(loader)?),
            },
            boot_device: lines.get("bootDevice").map(str::to_owned),
            hardware_config: match (nixos, hardware_config) {
                (true, Some(config)) => Some(config),
                (true, None) => return Err(eyre!("NixOS without its hardware configuration")),
                (false, _) => None,
            },
            time_zone: lines.get("timeZone").map(str::to_owned),
            login_user,
            main_user,
            cpus: lines.number("cpus")?,
            memory_mib: lines.number("memoryMib")?,
            nix_version,
        };

        let host_key = SshKey::parse(lines.required("hostKey")?)
            .map_err(|error| eyre!("the machine's /etc/ssh/ssh_host_ed25519_key.pub: {error}"))?;

        Ok(Self {
            facts,
            host_key,
            nix,
        })
    }
}

impl Facts {
    /// `NixOS 26.05 · x86_64-linux · EFI · 16 GiB · 8 cpus`, the reach row
    pub fn describe(&self) -> String {
        let os = match self.platform {
            Platform::Nixos => format!("NixOS {}", self.os_version),
            Platform::Darwin => format!("macOS {}", self.os_version),
        };
        let mut parts = vec![os, self.system.to_string()];
        if self.efi {
            parts.push("EFI".into());
        }
        parts.push(gibibytes(self.memory_mib));
        parts.push(format!("{} cpus", self.cpus));
        if self.nix_version.is_none() {
            parts.push("no nix yet".into());
        }
        parts.join(" · ")
    }
}

/// `5.8 GiB` under ten, `16 GiB` from there
fn gibibytes(mib: u64) -> String {
    let gib = mib as f64 / 1024.0;
    match gib < 10.0 {
        true => format!("{gib:.1} GiB"),
        false => format!("{gib:.0} GiB"),
    }
}

/// `nix (Nix) 2.31.2` or `nix (Determinate Nix 3.11.2) 2.31.2`: the last word
fn version_of(line: &str) -> String {
    line.split_whitespace().last().unwrap_or(line).to_owned()
}

fn boot_loader(name: &str) -> Result<BootLoader> {
    Ok(match name {
        "systemd-boot" => BootLoader::SystemdBoot,
        "grub" => BootLoader::Grub,
        "limine" => BootLoader::Limine,
        other => {
            return Err(eyre!(
                "the probe found a boot loader coeus does not know: {other}"
            ));
        }
    })
}

/// the probe's keys; an empty value is a key it could not learn
struct Lines<'a>(BTreeMap<&'a str, &'a str>);

impl<'a> Lines<'a> {
    fn of(output: &'a str) -> Self {
        Self(
            output
                .lines()
                .filter_map(|line| line.split_once('='))
                .map(|(key, value)| (key.trim(), value.trim()))
                .filter(|(_, value)| !value.is_empty())
                .collect(),
        )
    }

    fn get(&self, key: &str) -> Option<&'a str> {
        self.0.get(key).copied()
    }

    fn required(&self, key: &str) -> Result<&'a str> {
        self.get(key)
            .ok_or_else(|| eyre!("the probe could not learn the machine's {key}"))
    }

    fn number<T: std::str::FromStr>(&self, key: &str) -> Result<T> {
        let value = self.required(key)?;
        value
            .parse()
            .map_err(|_| eyre!("the probe's {key} is not a number: {value:?}"))
    }

    /// NixOS or macOS: coeus adopts machines, it does not install an OS
    fn platform(&self) -> Result<Platform> {
        match (self.required("kernel")?, self.get("osId")) {
            ("Darwin", _) => Ok(Platform::Darwin),
            ("Linux", Some("nixos")) => Ok(Platform::Nixos),
            ("Linux", _) => Err(eyre!(
                "the machine runs {}, not NixOS: install NixOS on it first, coeus only adopts \
                 a running NixOS or a Mac",
                self.get("osName").unwrap_or("a Linux that is not NixOS")
            )),
            (kernel, _) => Err(eyre!(
                "the machine runs {kernel}; coeus adopts NixOS and macOS"
            )),
        }
    }

    fn system(&self, platform: Platform) -> Result<System> {
        let machine = self.required("machine")?;
        Ok(match (platform, machine) {
            (Platform::Nixos, "x86_64") => System::X86_64Linux,
            (Platform::Nixos, "aarch64" | "arm64") => System::Aarch64Linux,
            (Platform::Darwin, "arm64" | "aarch64") => System::Aarch64Darwin,
            (Platform::Darwin, "x86_64") => System::X86_64Darwin,
            (_, other) => {
                return Err(eyre!(
                    "the machine's architecture {other} has no nix system coeus knows"
                ));
            }
        })
    }
}
