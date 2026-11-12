use tokio::process::Command;

use crate::{
    common::model::{Action, Platform, StorePath},
    platform::SystemRoot,
};

const PROFILE: &str = "/nix/var/nix/profiles/system";
/// what darwin-rebuild guarantees the activation script finds on `PATH`
const SYSTEM_DIRS: [&str; 4] = ["/usr/bin", "/bin", "/usr/sbin", "/sbin"];

/// whether the machine already is where `action` would take it with
/// `toplevel`, leaving nothing to build or activate
pub fn is_up_to_date(
    action: Action,
    toplevel: &StorePath,
    current: Option<&StorePath>,
    profile: Option<&StorePath>,
) -> bool {
    (action.activates() || action.sets_profile())
        && current == Some(toplevel)
        && profile == Some(toplevel)
}

/// switching a machine to a system, as root
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Activation {
    pub platform: Platform,
    pub action: Action,
    pub toplevel: StorePath,
    /// the profile generation that already holds `toplevel`, for a rollback
    pub generation: Option<u32>,
}

impl Activation {
    /// the activation that puts the machine back where it is now, taken
    /// before `action` changes it
    pub fn undo(platform: Platform, action: Action, system: &SystemRoot) -> Option<Self> {
        if action.sets_profile() {
            let generation = system.generation()?;
            return Some(Self {
                platform,
                action: Action::Switch,
                toplevel: system.generation_system(generation).ok()?,
                generation: Some(generation),
            });
        }

        Some(Self {
            platform,
            action,
            toplevel: system.current()?,
            generation: None,
        })
    }

    pub fn steps(&self) -> Vec<Step> {
        let profile = self.action.sets_profile().then(|| {
            let target = match self.generation {
                Some(generation) => ["--switch-generation".to_owned(), generation.to_string()],
                None => ["--set".to_owned(), self.toplevel.to_string()],
            };
            Step::new("nix-env").args(["-p", PROFILE]).args(target)
        });

        let activate = match self.platform {
            Platform::NixOs => Step::new(format!("{}/bin/switch-to-configuration", self.toplevel))
                .args([self.action.as_str()])
                .unset("NIXOS_INSTALL_BOOTLOADER"),
            Platform::Darwin => Step::new(format!("{}/activate", self.toplevel))
                .set("HOME", "/var/root")
                .set("NIX_REMOTE", "daemon")
                .set(
                    "PATH",
                    with_system_dirs(&std::env::var("PATH").unwrap_or_default()),
                ),
        };

        profile.into_iter().chain([activate]).collect()
    }

    /// what people read about where the machine goes
    pub fn describe(&self) -> String {
        match self.generation {
            Some(generation) => format!("generation {generation} ({})", self.toplevel.name()),
            None => self.toplevel.name().to_owned(),
        }
    }
}

/// one command of an activation, with the environment it differs in
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Step {
    pub program: String,
    pub args: Vec<String>,
    /// variables to set, or to remove when `None`
    pub env: Vec<(&'static str, Option<String>)>,
}

impl Step {
    fn new(program: impl Into<String>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
            env: Vec::new(),
        }
    }

    fn args<S: Into<String>>(mut self, args: impl IntoIterator<Item = S>) -> Self {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }

    fn set(mut self, name: &'static str, value: impl Into<String>) -> Self {
        self.env.push((name, Some(value.into())));
        self
    }

    fn unset(mut self, name: &'static str) -> Self {
        self.env.push((name, None));
        self
    }

    pub fn command(&self) -> Command {
        let mut command = Command::new(&self.program);
        command.args(&self.args);

        for (name, value) in &self.env {
            match value {
                Some(value) => command.env(name, value),
                None => command.env_remove(name),
            };
        }

        command
    }
}

fn with_system_dirs(path: &str) -> String {
    let mut dirs: Vec<&str> = path.split(':').filter(|dir| !dir.is_empty()).collect();
    for dir in SYSTEM_DIRS {
        if !dirs.contains(&dir) {
            dirs.push(dir);
        }
    }
    dirs.join(":")
}
