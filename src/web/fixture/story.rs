//! what the simulated fleet is made of: its machines and their history,
//! the packages it builds and what their builders print, and what the
//! rollout changes on each machine

use crate::common::model::{PackageChange, Platform};

pub const PACKAGES: [&str; 30] = [
    "rustc-1.84.0",
    "linux-6.12.9",
    "zfs-kernel-2.2.7-6.12.9",
    "systemd-257.2",
    "llvm-19.1.6",
    "coeusd-0.4.1",
    "openssl-3.3.2",
    "python3-3.12.7",
    "nodejs-22.11.0",
    "mesa-24.3.1",
    "glibc-2.40-66",
    "ghc-9.6.6",
    "cargo-1.84.0",
    "nix-2.25.3",
    "tailscale-1.78.1",
    "cloudflared-2024.12.1",
    "postgresql-17.2",
    "grafana-11.4.0",
    "firmware-linux-20241210",
    "perl-5.40.0",
    "bash-5.2p37",
    "curl-8.11.1",
    "sqlite-3.47.2",
    "go-1.23.4",
    "zstd-1.5.6",
    "util-linux-2.40.2",
    "gnupg-2.4.7",
    "ffmpeg-7.1",
    "neovim-0.10.3",
    "git-2.47.1",
];

/// what a package's builder prints, by the package's first word
pub fn printed(package: &str) -> &'static [&'static str] {
    match package.split('-').next().unwrap_or_default() {
        "rustc" => &[
            "Compiling serde v1.0.215",
            "Compiling syn v2.0.90",
            "Compiling proc-macro2 v1.0.92",
            "Building [=======>      ] 312/598: rustc_middle",
            "Compiling tokio v1.42.0",
        ],
        "cargo" => &[
            "Compiling libc v0.2.168",
            "Compiling curl-sys v0.4.78",
            "Finished `release` profile [optimized] target(s) in 1m 12s",
        ],
        "linux" => &[
            "CC [M]  drivers/net/wireguard/send.o",
            "LD [M]  net/wireguard/wireguard.ko",
            "CC      kernel/sched/fair.o",
            "AR      lib/lib.a",
            "CC [M]  fs/btrfs/inode.o",
        ],
        "zfs" => &[
            "CC [M]  module/zfs/arc.o",
            "CC [M]  module/zfs/dmu.o",
            "LD [M]  module/zfs/zfs.ko",
        ],
        "llvm" => &[
            "[1243/4410] Building CXX object lib/IR/CMakeFiles/LLVMCore.dir/Verifier.cpp.o",
            "[1250/4410] Linking CXX static library lib/libLLVMCore.a",
        ],
        _ => &[
            "checking for gcc... gcc",
            "make[2]: Entering directory '/build/source/src'",
            "installing",
            "post-installation fixup",
            "shrinking RPATHs of ELF executables and libraries",
            "patching script interpreter paths",
            "stripping (with command strip and flags -S -p)",
            "configure flags: --disable-static --prefix=/nix/store/…",
        ],
    }
}

/// what each machine ran, newest first: host | job | action | source |
/// origin | outcome | minutes ago it finished | seconds it ran
pub const HISTORY: &str = "
altar         | 42 | switch | main@91f0e2a     | actions · deploy #1179 | succeeded | 126 | 74
altar         | 41 | switch | snapshot 7c1e0aa | gustavo@altar | succeeded | 655 | 39
altar         | 40 | switch | main@e04b7d2     | actions · deploy #1171 | succeeded | 1428 | 81
altar         | 39 | switch | main@b71c2f9     | actions · deploy #1168 | failed evaluating: error: attribute 'coeus' missing | 1461 | 20
altar         | 38 | build  | snapshot 1d0e3b2 | gustavo@altar | succeeded | 2010 | 140
altar         | 37 | switch | main@a90de13     | actions · deploy #1160 | up_to_date | 2886 | 3
desktop-nixos | 88 | switch | main@2cbe6da     | actions · deploy #1184 | failed building: builder for '/nix/store/1q9xvk2…-zfs-kernel-2.2.7-6.12.9.drv' failed with exit code 2 | 3.1 | 74
desktop-nixos | 87 | switch | main@91f0e2a     | actions · deploy #1179 | succeeded | 130 | 305
desktop-nixos | 86 | test   | snapshot 7c1e0aa | gustavo@desktop-nixos | succeeded | 660 | 44
desktop-nixos | 85 | switch | generation 1361  | gustavo@altar | succeeded | 1400 | 21
desktop-nixos | 84 | switch | main@e04b7d2     | actions · deploy #1171 | rolled_back activating: display-manager.service failed | 1432 | 188
desktop-nixos | 83 | build  | main@e04b7d2     | gustavo@desktop-nixos | cancelled | 1440 | 61
desktop-nixos | 82 | switch | main@b71c2f9     | actions · deploy #1168 | failed evaluating: error: attribute 'coeus' missing | 1461 | 22
desktop-nixos | 81 | switch | deployed@a90de13 | catch-up | up_to_date | 2900 | 5
lab           | 60 | switch | main@91f0e2a     | actions · deploy #1179 | succeeded | 128 | 252
lab           | 59 | build  | snapshot 3f9a1c0 | gustavo@altar | succeeded | 301 | 110
lab           | 58 | switch | main@e04b7d2     | actions · deploy #1176 | superseded 60 | 306 | 0
lab           | 57 | test   | snapshot 3f9a1c0 | gustavo@altar | rolled_back verifying: coeusd.service did not become active | 372 | 96
lab           | 56 | switch | main@e04b7d2     | actions · deploy #1171 | succeeded | 1430 | 241
lab           | 55 | switch | main@b71c2f9     | actions · deploy #1168 | failed evaluating: error: attribute 'coeus' missing | 1460 | 19
lab           | 54 | boot   | main@b71c2f9     | gustavo@laptop-mac | cancelled | 1490 | 7
lab           | 53 | switch | deployed@a90de13 | catch-up | up_to_date | 2880 | 4
laptop-mac    | 36 | switch | deployed@e04b7d2 | catch-up | succeeded | 1180 | 102
laptop-mac    | 35 | switch | main@b71c2f9     | actions · deploy #1168 | failed evaluating: error: attribute 'coeus' missing | 1461 | 21
laptop-mac    | 34 | switch | deployed@a90de13 | catch-up | succeeded | 2600 | 88
laptop-mac    | 33 | switch | snapshot 1d0e3b2 | gustavo@laptop-mac | interrupted | 2700 | 302
oracle-1      | 24 | switch | main@2cbe6da     | actions · deploy #1184 | succeeded | 4.9 | 98
oracle-1      | 23 | switch | main@91f0e2a     | actions · deploy #1179 | succeeded | 131 | 87
oracle-1      | 22 | switch | main@e04b7d2     | actions · deploy #1171 | succeeded | 1433 | 92
oracle-1      | 21 | switch | main@b71c2f9     | actions · deploy #1168 | failed evaluating: error: attribute 'coeus' missing | 1461 | 17
oracle-1      | 20 | switch | main@a90de13     | actions · deploy #1160 | up_to_date | 2890 | 3
oracle-1      | 19 | switch | main@0c4e8f1     | actions · deploy #1158 | succeeded | 4296 | 64
oracle-1      | 18 | switch | main@0c4e8f1     | actions · deploy #1158 | interrupted | 4300 | 51
oracle-2      | 31 | boot   | main@2cbe6da     | actions · deploy #1184 | succeeded | 2.2 | 71
oracle-2      | 30 | boot   | main@91f0e2a     | actions · deploy #1179 | succeeded | 129 | 66
oracle-2      | 29 | switch | snapshot 7c1e0aa | gustavo@altar | succeeded | 640 | 58
oracle-2      | 28 | boot   | main@e04b7d2     | actions · deploy #1171 | superseded 29 | 700 | 0
oracle-2      | 27 | boot   | main@b71c2f9     | actions · deploy #1168 | failed evaluating: error: attribute 'coeus' missing | 1461 | 18
oracle-2      | 26 | boot   | main@a90de13     | actions · deploy #1160 | up_to_date | 2890 | 3
";

/// each machine as the story starts: platform, system, generation, and
/// what its running system was deployed from
pub const MACHINES: [(Platform, &str, u32, &str); 6] = [
    (Platform::Darwin, "aarch64-darwin", 412, "main@91f0e2a"),
    (Platform::NixOs, "x86_64-linux", 1366, "main@91f0e2a"),
    (Platform::NixOs, "x86_64-linux", 1371, "main@91f0e2a"),
    (Platform::Darwin, "aarch64-darwin", 388, "main@e04b7d2"),
    (Platform::NixOs, "aarch64-linux", 804, "main@2cbe6da"),
    (Platform::NixOs, "aarch64-linux", 798, "main@2cbe6da"),
];

/// `[name, before, after, MiB]` as a package change
fn change(name: &str, before: Option<&str>, after: Option<&str>, mib: f64) -> PackageChange {
    PackageChange {
        name: name.to_owned(),
        before: before.into_iter().map(str::to_owned).collect(),
        after: after.into_iter().map(str::to_owned).collect(),
        size_delta: (mib * 1_048_576.0) as i64,
    }
}

pub fn diffs() -> [Vec<PackageChange>; 6] {
    let linux = || change("linux", Some("6.12.8"), Some("6.12.9"), 0.4);
    let coeusd = || change("coeusd", Some("0.4.0"), Some("0.4.1"), 0.1);
    let openssl = || change("openssl", Some("3.3.1"), Some("3.3.2"), 0.02);
    let systemd = || change("systemd", Some("256.8"), Some("257.2"), 1.3);
    let zfs = || change("zfs-kernel", Some("2.2.6"), Some("2.2.7"), 0.3);
    let ghostty = || change("ghostty", None, Some("1.0.1"), 38.4);
    [
        vec![
            coeusd(),
            change("nix", Some("2.24.11"), Some("2.25.3"), 0.6),
            change("neovim", Some("0.10.2"), Some("0.10.3"), 0.1),
            change("git", Some("2.47.0"), Some("2.47.1"), 0.0),
            ghostty(),
        ],
        vec![
            linux(),
            zfs(),
            change("mesa", Some("24.2.8"), Some("24.3.1"), 3.4),
            change("firefox", Some("132.0.2"), Some("133.0"), 1.9),
            coeusd(),
        ],
        vec![
            linux(),
            coeusd(),
            systemd(),
            change("rustc", Some("1.83.0"), Some("1.84.0"), 12.1),
            zfs(),
            change("python3.11", Some("3.11.10"), None, -48.2),
            change("python3.12", None, Some("3.12.7"), 51.6),
            openssl(),
        ],
        vec![
            coeusd(),
            change("nix", Some("2.24.10"), Some("2.25.3"), 0.7),
            change("git", Some("2.46.2"), Some("2.47.1"), 0.1),
            change("ffmpeg", Some("7.0.2"), Some("7.1"), 4.8),
            ghostty(),
        ],
        vec![
            linux(),
            coeusd(),
            systemd(),
            openssl(),
            change("tailscale", Some("1.76.6"), Some("1.78.1"), 2.2),
        ],
        vec![linux(), coeusd(), systemd(), openssl()],
    ]
}
