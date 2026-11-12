use super::strip_ansi;
use crate::common::model::{ClosureDiff, PackageChange};

impl ClosureDiff {
    /// reads the output of `nix store diff-closures`, skipping lines it does
    /// not recognise
    pub fn parse(output: &str) -> Self {
        Self {
            changes: output.lines().filter_map(PackageChange::parse).collect(),
        }
    }
}

impl PackageChange {
    /// reads one line of `nix store diff-closures`, in either format nix has
    /// printed: `git: 2.55.0 → 2.56.0, 1.5 MiB`, `age: 1.3.2 added`,
    /// `bash: 5.3p15 removed, -8.1 MiB`, or the older `hello: ∅ → ε, +13.9 KiB`.
    /// a package without a version has an empty one.
    pub fn parse(line: &str) -> Option<Self> {
        let line = strip_ansi(line);
        let (name, rest) = line.trim().split_once(": ")?;

        let (versions, size_delta) = size(rest)
            .map(|delta| ("", delta))
            .or_else(|| {
                let (versions, last) = rest.rsplit_once(", ")?;
                Some((versions, size(last)?))
            })
            .unwrap_or((rest, 0));

        let (before, after) = if versions.is_empty() {
            (Vec::new(), Vec::new())
        } else if let Some((before, after)) = versions.split_once(" → ") {
            (version_list(before), version_list(after))
        } else if let Some(added) = versions.strip_suffix(" added") {
            (Vec::new(), version_list(added))
        } else {
            (version_list(versions.strip_suffix(" removed")?), Vec::new())
        };

        Some(Self {
            name: name.to_owned(),
            before,
            after,
            size_delta,
        })
    }
}

fn version_list(list: &str) -> Vec<String> {
    if list == "∅" {
        return Vec::new();
    }

    list.split(", ")
        .map(|version| match version {
            "ε" | "(no version)" => String::new(),
            version => version.to_owned(),
        })
        .collect()
}

/// `1.5 MiB`, `-15.2 KiB` or `+13.9 KiB`, in bytes
fn size(text: &str) -> Option<i64> {
    const UNITS: [&str; 9] = [
        "KiB", "MiB", "GiB", "TiB", "PiB", "EiB", "ZiB", "YiB", "RiB",
    ];

    let (number, unit) = text.split_once(' ')?;
    let power = UNITS.iter().position(|known| *known == unit)?;
    let value: f64 = number.parse().ok()?;

    Some((value * 1024_f64.powi(power as i32 + 1)).round() as i64)
}
