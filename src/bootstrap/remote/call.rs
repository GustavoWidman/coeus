//! every command coeus runs on a machine, so none is built from a string
//! anywhere else. `argv` is pure and every variant's is pinned by a test.

use std::path::PathBuf;

use crate::{
    bootstrap::SshKey,
    common::model::{Hostname, NarHash, OrderId, StorePath},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Call {
    /// `sh -s` reading probe.sh on stdin, its `key=value` lines on stdout.
    /// the login user is passed in, since the probe may run as root
    Probe { login_user: String },
    /// `nixos-generate-config --show-hardware-config`, as root
    HardwareConfig,
    /// Determinate's installer, as root, without questions
    InstallNix,
    /// `/nix/nix-installer repair`, for an install a crash cut short
    RepairNix,
    /// moves `/etc/bashrc`, `/etc/zshrc` and `/etc/zprofile` to
    /// `.before-nix-darwin` when nix-darwin did not write them; prints the
    /// ones it moved
    MoveEtcAside,
    /// adds the session key to root's `authorized_keys` unless it is there
    InstallSessionKey { key: SshKey, host: Hostname },
    /// removes every line carrying the session key
    RemoveSessionKey { key: SshKey },
    /// `nix build --json --log-format internal-json <flake>#<attribute>` as
    /// root, the coeus the machine's configuration pins
    BuildCoeus {
        source: StorePath,
        attribute: String,
    },
    /// `<coeus>/bin/coeusd adopt …`, as root
    Adopt(AdoptCall),
    /// `<coeus>/bin/coeusd adopt --status --nar-hash … --source …
    /// --hostname …`, as root: the source and hostname name the state
    /// directory the configuration gives coeusd
    AdoptStatus {
        coeus: StorePath,
        nar_hash: NarHash,
        source: PathBuf,
        hostname: Hostname,
    },
    /// `tailscale status --json`, when coeusd stays silent
    Tailscale,
    /// the system the machine runs and its boot id, as `system=` and
    /// `boot=` lines
    Running,
    /// `systemctl reboot`, as root
    Reboot,
    /// System Settings' Full Disk Access pane, and `launcher` shown in
    /// Finder, on the screen of whoever is logged in at the Mac
    OpenFullDiskAccess { launcher: String },
}

/// the flags of `coeusd adopt`, the one interface between this CLI and the
/// fleet's coeus before a coeusd runs: kept small, it outlives version skew
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdoptCall {
    pub coeus: StorePath,
    pub source: PathBuf,
    pub nar_hash: NarHash,
    pub label: String,
    pub hostname: Hostname,
    pub order: OrderId,
    /// admit the order as a boot, for a NixOS that refused a live switch
    pub boot: bool,
    pub install_bootloader: bool,
    /// stream the job's events from this log offset
    pub follow: Option<u64>,
}

/// where nix is when the login shell did not say: Determinate's profile on
/// a Mac, the system's on NixOS. root's ssh session on a Mac has neither.
const NIX_PATH: &str =
    r#"PATH="/nix/var/nix/profiles/default/bin:/run/current-system/sw/bin:$PATH" exec "$@""#;

const INSTALL_NIX: &str = "curl --proto '=https' --tlsv1.2 -fsSL https://install.determinate.systems/nix | sh -s -- install --no-confirm";

/// `$1` the whole `authorized_keys` line, `$2` the key it carries
const INSTALL_SESSION_KEY: &str = r#"set -eu
umask 077
dir=~root/.ssh
file="$dir/authorized_keys"
mkdir -p "$dir"
touch "$file"
grep -qF "$2" "$file" && exit 0
if [ -s "$file" ] && [ -n "$(tail -c 1 "$file")" ]; then echo >> "$file"; fi
printf '%s\n' "$1" >> "$file"
"#;

/// `$1` the key; every other line stays as it was. grep exits 1 when no
/// other line is left; any other failure, or no copy written, leaves the
/// file as it was rather than replace it with a truncated copy
const REMOVE_SESSION_KEY: &str = r#"set -eu
file=~root/.ssh/authorized_keys
[ -f "$file" ] || exit 0
grep -qF "$1" "$file" || exit 0
umask 077
rm -f "$file.coeus"
kept=0
grep -vF "$1" "$file" > "$file.coeus" || kept=$?
if [ "$kept" -gt 1 ] || [ ! -f "$file.coeus" ]; then
  rm -f "$file.coeus"
  echo "could not rewrite $file without the session key" >&2
  exit 1
fi
mv "$file.coeus" "$file"
"#;

/// nix-darwin refuses to replace an `/etc` file it does not know, and asks
/// for exactly this rename: the shell files and `nix.custom.conf` the Nix
/// installer writes are such files. a copy already aside keeps its name,
/// since it may be the machine's original (macOS images ship one): a
/// different file goes to the next free `.before-nix-darwin.<n>`, and one
/// equal to a copy aside is that move done before. nothing is deleted that
/// is not kept somewhere aside. prints `<file> to <where it is kept>`.
const MOVE_ETC_ASIDE: &str = r#"set -eu
for file in /etc/bashrc /etc/zshrc /etc/zprofile /etc/nix/nix.custom.conf; do
  [ -e "$file" ] || continue
  case "$(readlink "$file" || true)" in /etc/static/*) continue ;; esac
  aside="$file.before-nix-darwin"
  target=$aside
  n=0
  while [ -e "$target" ] && ! cmp -s "$file" "$target"; do
    n=$((n + 1))
    target="$aside.$n"
  done
  if [ -e "$target" ]; then rm "$file"; else mv "$file" "$target"; fi
  echo "$file to $target"
done
"#;

/// `$1` the launcher. nothing to open when nobody is logged in at the Mac
const OPEN_FULL_DISK_ACCESS: &str = r#"user=$(stat -f %Su /dev/console)
[ "$user" != root ] || exit 0
uid=$(id -u "$user")
launchctl asuser "$uid" sudo -u "$user" open "x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles"
launchctl asuser "$uid" sudo -u "$user" open -R "$1"
"#;

const RUNNING: &str = r#"printf 'system=%s\n' "$(readlink -f /run/current-system 2>/dev/null)"
printf 'boot=%s\n' "$(cat /proc/sys/kernel/random/boot_id 2>/dev/null)"
"#;

const TAILSCALE: &str = r#"for tailscale in tailscale /run/current-system/sw/bin/tailscale /Applications/Tailscale.app/Contents/MacOS/Tailscale; do
  command -v "$tailscale" >/dev/null 2>&1 && exec "$tailscale" status --json
done
echo "tailscale is not installed" >&2
exit 127
"#;

impl Call {
    pub fn argv(&self) -> Vec<String> {
        match self {
            Self::Probe { login_user } => words(["sh", "-s", "--", login_user]),
            Self::HardwareConfig => with_nix(["nixos-generate-config", "--show-hardware-config"]),
            Self::InstallNix => words(["sh", "-c", INSTALL_NIX]),
            Self::RepairNix => words(["/nix/nix-installer", "repair", "--no-confirm"]),
            Self::MoveEtcAside => words(["sh", "-c", MOVE_ETC_ASIDE]),
            Self::InstallSessionKey { key, host } => words([
                "sh",
                "-c",
                INSTALL_SESSION_KEY,
                "sh",
                &format!("{key} coeus-bootstrap@{host}"),
                &key.to_string(),
            ]),
            Self::RemoveSessionKey { key } => {
                words(["sh", "-c", REMOVE_SESSION_KEY, "sh", &key.to_string()])
            }
            Self::BuildCoeus { source, attribute } => with_nix([
                "nix",
                "--extra-experimental-features",
                crate::nix::FLAKE_FEATURES,
                "build",
                "--json",
                "--no-link",
                "--log-format",
                "internal-json",
                "--option",
                "accept-flake-config",
                "true",
                "--no-write-lock-file",
                &format!("path:{source}#{attribute}"),
            ]),
            Self::Adopt(adopt) => with_nix(adopt.argv()),
            Self::AdoptStatus {
                coeus,
                nar_hash,
                source,
                hostname,
            } => with_nix([
                &coeusd(coeus),
                "adopt",
                "--status",
                "--nar-hash",
                nar_hash.as_str(),
                "--source",
                &source.display().to_string(),
                "--hostname",
                hostname.as_str(),
            ]),
            Self::Tailscale => words(["sh", "-c", TAILSCALE]),
            Self::Running => words(["sh", "-c", RUNNING]),
            Self::Reboot => with_nix(["systemctl", "reboot"]),
            Self::OpenFullDiskAccess { launcher } => {
                words(["sh", "-c", OPEN_FULL_DISK_ACCESS, "sh", launcher])
            }
        }
    }

    /// whether it must run as root
    pub fn privileged(&self) -> bool {
        !matches!(self, Self::Probe { .. } | Self::Tailscale | Self::Running)
    }

    /// what the call reads on stdin, beyond a sudo password
    pub fn stdin(&self) -> Option<&'static [u8]> {
        match self {
            Self::Probe { .. } => Some(include_bytes!("probe.sh")),
            _ => None,
        }
    }
}

impl AdoptCall {
    fn argv(&self) -> Vec<String> {
        let mut argv = words([
            &coeusd(&self.coeus),
            "adopt",
            "--source",
            &self.source.display().to_string(),
            "--nar-hash",
            self.nar_hash.as_str(),
            "--label",
            &self.label,
            "--hostname",
            self.hostname.as_str(),
            "--order",
            &self.order.to_string(),
        ]);
        if self.boot {
            argv.push("--boot".into());
        }
        if self.install_bootloader {
            argv.push("--install-bootloader".into());
        }
        if let Some(offset) = self.follow {
            argv.extend(["--follow".into(), offset.to_string()]);
        }
        argv
    }
}

fn coeusd(coeus: &StorePath) -> String {
    format!("{coeus}/bin/coeusd")
}

fn words<'a>(words: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    words.into_iter().map(str::to_owned).collect()
}

/// `argv` with nix's usual places on `PATH`
fn with_nix<S: AsRef<str>>(argv: impl IntoIterator<Item = S>) -> Vec<String> {
    let mut words = words(["sh", "-c", NIX_PATH, "sh"]);
    words.extend(argv.into_iter().map(|word| word.as_ref().to_owned()));
    words
}
