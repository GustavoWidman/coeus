use crate::{common::model::Source, job::Target};

/// how a runner gets the system a source names
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Recipe {
    /// evaluate and build it from this locked flake reference
    Flake(String),
    /// take it from this generation of the system profile
    Generation(u32),
}

impl Target {
    pub fn recipe(&self, source: &Source) -> Recipe {
        match source {
            Source::Git { rev, git_ref } => {
                let separator = if self.flake.contains('?') { '&' } else { '?' };
                let git_ref = git_ref.as_ref().map(|git_ref| format!("&ref={git_ref}"));
                Recipe::Flake(format!(
                    "{}{separator}rev={rev}{}",
                    self.flake,
                    git_ref.unwrap_or_default()
                ))
            }
            Source::Snapshot { nar_hash, .. } => Recipe::Flake(format!(
                "path:{}?narHash={}",
                self.sources.join(nar_hash.file_name()).display(),
                percent_encode(nar_hash.as_str()),
            )),
            Source::Generation(number) => Recipe::Generation(*number),
        }
    }

    /// this machine's system in `flake`
    pub fn toplevel(&self, flake: &str) -> String {
        let hostname = self.hostname.as_str();
        let plain = hostname
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'));
        let name = match plain {
            true => hostname.to_owned(),
            false => format!("\"{hostname}\""),
        };

        format!(
            "{flake}#{}.{name}.config.system.build.toplevel",
            self.platform.configurations()
        )
    }
}

/// base64's `+`, `/` and `=` mean something else in a URL query
fn percent_encode(value: &str) -> String {
    value
        .replace('+', "%2B")
        .replace('/', "%2F")
        .replace('=', "%3D")
}
