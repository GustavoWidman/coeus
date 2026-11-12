use std::time::Duration;

use crate::common::model::{Progress, Tally, Timestamp};

/// `42s`, `3m12s`, `1h04m`
pub fn duration(duration: Duration) -> String {
    let seconds = duration.as_secs();
    match seconds {
        0..60 => format!("{seconds}s"),
        60..3600 => format!("{}m{:02}s", seconds / 60, seconds % 60),
        _ => format!("{}h{:02}m", seconds / 3600, seconds % 3600 / 60),
    }
}

/// `just now`, `42s ago`, `3m ago`, `2h ago`, `5d ago`
pub fn ago(at: Timestamp, now: Timestamp) -> String {
    let seconds = at.until(now).as_secs();
    match seconds {
        0..5 => "just now".to_owned(),
        5..60 => format!("{seconds}s ago"),
        60..3600 => format!("{}m ago", seconds / 60),
        3600..86400 => format!("{}h ago", seconds / 3600),
        _ => format!("{}d ago", seconds / 86400),
    }
}

/// the binary unit a byte count reads best in
#[derive(Clone, Copy)]
struct Unit {
    name: &'static str,
    size: f64,
}

impl Unit {
    const ALL: [Self; 4] = [
        Self {
            name: "B",
            size: 1.0,
        },
        Self {
            name: "KiB",
            size: 1024.0,
        },
        Self {
            name: "MiB",
            size: 1024.0 * 1024.0,
        },
        Self {
            name: "GiB",
            size: 1024.0 * 1024.0 * 1024.0,
        },
    ];

    fn of(bytes: u64) -> Self {
        Self::ALL
            .into_iter()
            .rev()
            .find(|unit| bytes as f64 >= unit.size)
            .unwrap_or(Self::ALL[0])
    }

    fn amount(self, bytes: u64) -> String {
        match self.name {
            "B" => bytes.to_string(),
            _ => format!("{:.1}", bytes as f64 / self.size),
        }
    }
}

/// `512 B`, `12.0 KiB`, `3.4 GiB`
pub fn bytes(bytes: u64) -> String {
    let unit = Unit::of(bytes);
    format!("{} {}", unit.amount(bytes), unit.name)
}

/// `1.2/3.4 GiB`, both in the unit of the total
pub fn bytes_of(done: u64, total: u64) -> String {
    let unit = Unit::of(total);
    format!("{}/{} {}", unit.amount(done), unit.amount(total), unit.name)
}

/// `+1.2 MiB`, `-300.0 KiB`, or `0 B` without a sign
pub fn size_delta(delta: i64) -> String {
    let sign = match delta.signum() {
        -1 => "-",
        1 => "+",
        _ => "",
    };
    format!("{sign}{}", bytes(delta.unsigned_abs()))
}

/// `42%`
pub fn percent(fraction: f64) -> String {
    format!("{:.0}%", (fraction * 100.0).floor())
}

/// `142/225 built · 12/40 fetched · 1.2/3.4 GiB`, leaving out what nix has
/// not announced
pub fn counts(progress: &Progress) -> String {
    let tally = |tally: &Tally, verb: &str| {
        (!tally.is_empty()).then(|| format!("{}/{} {verb}", tally.done, tally.expected))
    };

    [
        tally(&progress.builds, "built"),
        tally(&progress.downloads, "fetched"),
        (progress.expected_bytes > 0)
            .then(|| bytes_of(progress.downloaded_bytes, progress.expected_bytes)),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" · ")
}

/// the readable part of a derivation name: `firefox-121.0` from
/// `/nix/store/…-firefox-121.0.drv`
pub fn derivation(name: &str) -> &str {
    let name = name.rsplit('/').next().unwrap_or(name);
    let name = name.strip_suffix(".drv").unwrap_or(name);
    match name.split_once('-') {
        Some((hash, rest)) if hash.len() == 32 => rest,
        _ => name,
    }
}
