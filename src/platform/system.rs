use std::{
    fs,
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

use eyre::{Result, eyre};

use crate::common::model::{Generation, Platform, StorePath, Timestamp};

/// what the system profile and `/run` say about the machine right now
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct SystemFacts {
    pub current: Option<StorePath>,
    pub generation: Option<u32>,
    /// the profile points somewhere other than the running system
    pub pending_boot: bool,
    /// when the profile's current generation was made, the best record of
    /// when the machine last switched, whoever did it
    pub switched_at: Option<Timestamp>,
    /// the running system boots a different kernel than the booted one
    pub reboot_required: bool,
}

/// where a machine keeps its systems: the system profile with its numbered
/// generations, and the running and booted systems under `/run`. read through
/// symlinks only, never through the store paths they name.
#[derive(Clone, Debug)]
pub struct SystemRoot {
    profiles: PathBuf,
    run: PathBuf,
}

impl Default for SystemRoot {
    fn default() -> Self {
        Self::at(Path::new("/"))
    }
}

impl SystemRoot {
    /// what decides whether a machine needs a reboot to run its current system
    const BOOT_FILES: [&'static str; 3] = ["kernel", "initrd", "kernel-modules"];

    pub fn at(root: &Path) -> Self {
        Self {
            profiles: root.join("nix/var/nix/profiles"),
            run: root.join("run"),
        }
    }

    pub fn profile(&self) -> PathBuf {
        self.profiles.join("system")
    }

    pub fn generation_link(&self, number: u32) -> PathBuf {
        self.profiles.join(format!("system-{number}-link"))
    }

    /// the running system
    pub fn current(&self) -> Option<StorePath> {
        resolve(&self.run.join("current-system"))
    }

    /// the system the machine booted, NixOS only
    pub fn booted(&self) -> Option<StorePath> {
        resolve(&self.run.join("booted-system"))
    }

    /// the system the profile points at, the next boot's default
    pub fn profile_system(&self) -> Option<StorePath> {
        resolve(&self.profile())
    }

    /// the profile's generation number, from its `system-<n>-link` target
    pub fn generation(&self) -> Option<u32> {
        let target = fs::read_link(self.profile()).ok()?;
        generation_number(target.file_name()?.to_str()?)
    }

    pub fn generation_system(&self, number: u32) -> Result<StorePath> {
        resolve(&self.generation_link(number))
            .ok_or_else(|| eyre!("generation {number} of the system profile does not exist"))
    }

    pub fn facts(&self, platform: Platform) -> SystemFacts {
        let current = self.current();
        let profile = self.profile_system();

        SystemFacts {
            pending_boot: profile.is_some() && profile != current,
            reboot_required: platform == Platform::NixOs && self.boots_differently(),
            generation: self.generation(),
            switched_at: self.generation().and_then(|number| self.made_at(number)),
            current,
        }
    }

    fn made_at(&self, number: u32) -> Option<Timestamp> {
        let modified = self
            .generation_link(number)
            .symlink_metadata()
            .ok()?
            .modified()
            .ok()?;
        let millis = modified.duration_since(UNIX_EPOCH).ok()?.as_millis();
        Some(Timestamp(millis as i64))
    }

    /// every generation of the system profile, newest first
    pub fn generations(&self) -> Result<Vec<Generation>> {
        let current = self.current();
        let booted = self.booted();

        let mut generations: Vec<Generation> = fs::read_dir(&self.profiles)?
            .filter_map(|entry| {
                let entry = entry.ok()?;
                let number = generation_number(entry.file_name().to_str()?)?;
                let toplevel = resolve(&entry.path())?;
                let modified = entry.path().symlink_metadata().ok()?.modified().ok()?;
                let millis = modified.duration_since(UNIX_EPOCH).ok()?.as_millis();

                Some(Generation {
                    number,
                    created_at: Timestamp(i64::try_from(millis).ok()?),
                    current: current.as_ref() == Some(&toplevel),
                    booted: booted.as_ref() == Some(&toplevel),
                    toplevel,
                })
            })
            .collect();

        generations.sort_unstable_by_key(|generation| std::cmp::Reverse(generation.number));
        Ok(generations)
    }

    fn boots_differently(&self) -> bool {
        let booted = self.run.join("booted-system");
        let current = self.run.join("current-system");

        booted.exists()
            && Self::BOOT_FILES.iter().any(|file| {
                fs::read_link(booted.join(file)).ok() != fs::read_link(current.join(file)).ok()
            })
    }
}

/// `system-10-link` is generation 10
fn generation_number(name: &str) -> Option<u32> {
    name.strip_prefix("system-")?
        .strip_suffix("-link")?
        .parse()
        .ok()
}

/// follows a chain of symlinks to the store path it ends in, without reading
/// the store path itself
fn resolve(link: &Path) -> Option<StorePath> {
    let mut path = link.to_owned();

    for _ in 0..8 {
        let target = fs::read_link(&path).ok()?;
        let target = match target.is_absolute() {
            true => target,
            false => path.parent()?.join(target),
        };

        if let Ok(store_path) = target.to_str()?.parse() {
            return Some(store_path);
        }
        path = target;
    }

    None
}
