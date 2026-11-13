use std::{ffi::OsString, os::unix::fs::symlink, path::Path};

use super::{launcher::runner_path, *};
use crate::common::model::{Action, JobId, Platform};

fn system(name: &str) -> String {
    format!("/nix/store/{}-{name}", "a".repeat(32))
}

/// a machine on generation 10 still running generation 9, as after `boot`
fn profiles(root: &Path) {
    let profiles = root.join("nix/var/nix/profiles");
    std::fs::create_dir_all(&profiles).unwrap();
    std::fs::create_dir_all(root.join("run")).unwrap();

    symlink(system("nixos-system-lab-9"), profiles.join("system-9-link")).unwrap();
    symlink(
        system("nixos-system-lab-10"),
        profiles.join("system-10-link"),
    )
    .unwrap();
    symlink("system-10-link", profiles.join("system")).unwrap();
    symlink(
        system("nixos-system-lab-9"),
        root.join("run/current-system"),
    )
    .unwrap();
    symlink(system("nixos-system-lab-9"), root.join("run/booted-system")).unwrap();
}

#[test]
fn facts_follow_the_profile_and_the_running_system() {
    let root = tempfile::tempdir().unwrap();
    profiles(root.path());

    let facts = SystemRoot::at(root.path()).facts(Platform::NixOs);
    assert_eq!(
        facts.current,
        Some(system("nixos-system-lab-9").parse().unwrap())
    );
    assert_eq!(facts.generation, Some(10));
    assert!(facts.pending_boot);
    assert!(!facts.reboot_required);
}

#[test]
fn a_fresh_machine_has_no_facts() {
    let root = tempfile::tempdir().unwrap();
    let facts = SystemRoot::at(root.path()).facts(Platform::NixOs);

    assert_eq!(facts.current, None);
    assert_eq!(facts.generation, None);
    assert!(!facts.pending_boot);
}

#[test]
fn lists_generations_newest_first() {
    let root = tempfile::tempdir().unwrap();
    profiles(root.path());

    let generations = SystemRoot::at(root.path()).generations().unwrap();
    let numbers: Vec<u32> = generations
        .iter()
        .map(|generation| generation.number)
        .collect();
    assert_eq!(numbers, [10, 9]);

    assert!(generations[1].current && generations[1].booted);
    assert!(!generations[0].current && !generations[0].booted);
    assert_eq!(generations[0].toplevel.name(), "nixos-system-lab-10");
    assert!(generations[0].created_at.0 > 0);

    let root = SystemRoot::at(root.path());
    assert_eq!(
        root.generation_system(9).unwrap().name(),
        "nixos-system-lab-9"
    );
    assert!(root.generation_system(3).is_err());
}

/// generations 10 and 12 hold the same system, as after deploying a commit
/// again; the profile points at `profile`
fn repeated(root: &Path, profile: u32) {
    let profiles = root.join("nix/var/nix/profiles");
    std::fs::create_dir_all(&profiles).unwrap();
    std::fs::create_dir_all(root.join("run")).unwrap();

    for (number, name) in [(10, "a"), (11, "b"), (12, "a")] {
        symlink(
            system(&format!("darwin-system-{name}")),
            profiles.join(format!("system-{number}-link")),
        )
        .unwrap();
    }
    symlink(format!("system-{profile}-link"), profiles.join("system")).unwrap();
    symlink(system("darwin-system-a"), root.join("run/current-system")).unwrap();
}

fn current(root: &Path) -> Vec<u32> {
    SystemRoot::at(root)
        .generations()
        .unwrap()
        .iter()
        .filter(|generation| generation.current)
        .map(|generation| generation.number)
        .collect()
}

#[test]
fn only_one_generation_runs_when_several_hold_the_running_system() {
    let root = tempfile::tempdir().unwrap();
    repeated(root.path(), 12);
    assert_eq!(current(root.path()), [12]);
}

#[test]
fn the_profile_names_the_running_generation_after_switching_back() {
    let root = tempfile::tempdir().unwrap();
    repeated(root.path(), 10);
    assert_eq!(current(root.path()), [10]);
}

#[test]
fn a_new_kernel_requires_a_reboot_on_nixos() {
    let root = tempfile::tempdir().unwrap();
    let run = root.path().join("run");
    for (system_dir, kernel) in [
        ("booted-system", "linux-6.12"),
        ("current-system", "linux-6.13"),
    ] {
        let dir = run.join(system_dir);
        std::fs::create_dir_all(&dir).unwrap();
        symlink(system(kernel) + "/bzImage", dir.join("kernel")).unwrap();
        symlink(system("initrd"), dir.join("initrd")).unwrap();
    }

    let root = SystemRoot::at(root.path());
    assert!(root.facts(Platform::NixOs).reboot_required);
    assert!(!root.facts(Platform::Darwin).reboot_required);

    std::fs::remove_file(run.join("current-system/kernel")).unwrap();
    symlink(
        system("linux-6.12") + "/bzImage",
        run.join("current-system/kernel"),
    )
    .unwrap();
    assert!(!root.facts(Platform::NixOs).reboot_required);
}

#[test]
fn darwin_only_switches_and_builds() {
    assert!(Platform::Darwin.supports(Action::Switch));
    assert!(Platform::Darwin.supports(Action::Build));
    assert!(!Platform::Darwin.supports(Action::Boot));
    assert!(!Platform::Darwin.supports(Action::Test));
    assert!(Platform::NixOs.supports(Action::Boot));
}

fn runner_command() -> Vec<OsString> {
    [
        "/nix/store/x-coeus/bin/coeusd",
        "run",
        "/var/lib/coeus/jobs/000042",
    ]
    .map(OsString::from)
    .to_vec()
}

#[test]
fn systemd_runs_a_collected_transient_service_per_job() {
    let environment = [("NIX_REMOTE".to_owned(), "daemon".to_owned())];
    let args = systemd_run_args(Task::Job(JobId(42)), &environment, &runner_command());

    assert_eq!(
        args,
        [
            "--unit=coeus-job-42",
            "--collect",
            "--quiet",
            "--service-type=exec",
            "--setenv=NIX_REMOTE=daemon",
            "--",
            "/nix/store/x-coeus/bin/coeusd",
            "run",
            "/var/lib/coeus/jobs/000042",
        ]
    );
}

#[test]
fn runners_find_the_running_systems_tools() {
    assert_eq!(
        runner_path(Some("/nix/store/x-nix/bin:/nix/store/x-git/bin")),
        "/nix/store/x-nix/bin:/nix/store/x-git/bin:/run/current-system/sw/bin"
    );
    assert_eq!(
        runner_path(Some("/run/wrappers/bin:/run/current-system/sw/bin")),
        "/run/wrappers/bin:/run/current-system/sw/bin"
    );
    assert_eq!(runner_path(None), "/run/current-system/sw/bin");
}

#[test]
fn launchd_runs_the_job_once_under_the_launcher() {
    let label = Task::Job(JobId(42)).label();
    let arguments: Vec<OsString> = [OsString::from("/usr/bin/time")]
        .into_iter()
        .chain(runner_command())
        .collect();
    let environment = [("HOME".to_owned(), "/var/root".to_owned())];
    let plist = Plist {
        label: &label,
        arguments: &arguments,
        environment: &environment,
        log: Path::new("/var/lib/coeus/jobs/000042/runner.log"),
    }
    .render();

    let expected = [
        "<key>Label</key>\n\t<string>dev.coeus.job.42</string>",
        "<array>\n\t\t<string>/usr/bin/time</string>\n\t\t<string>/nix/store/x-coeus/bin/coeusd</string>\n\t\t<string>run</string>\n\t\t<string>/var/lib/coeus/jobs/000042</string>\n\t</array>",
        "<key>RunAtLoad</key>\n\t<true/>",
        "<key>KeepAlive</key>\n\t<false/>",
        "<key>AbandonProcessGroup</key>\n\t<true/>",
        "<key>HOME</key>\n\t\t<string>/var/root</string>",
        "<key>StandardErrorPath</key>\n\t<string>/var/lib/coeus/jobs/000042/runner.log</string>",
    ];
    for fragment in expected {
        assert!(plist.contains(fragment), "missing {fragment:?} in\n{plist}");
    }
}

#[test]
fn plists_escape_their_strings() {
    let arguments = [OsString::from("a<b&c")];
    let plist = Plist {
        label: "dev.coeus.probe",
        arguments: &arguments,
        environment: &[],
        log: Path::new("/tmp/log"),
    }
    .render();

    assert!(plist.contains("<string>a&lt;b&amp;c</string>"));
}

#[cfg(target_os = "macos")]
#[test]
fn plutil_accepts_the_plist() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("job.plist");
    let arguments = runner_command();
    let environment = [("PATH".to_owned(), "/usr/bin:/bin".to_owned())];
    let plist = Plist {
        label: "dev.coeus.job.42",
        arguments: &arguments,
        environment: &environment,
        log: Path::new("/tmp/runner.log"),
    };
    std::fs::write(&path, plist.render()).unwrap();

    let status = std::process::Command::new("/usr/bin/plutil")
        .arg("-lint")
        .arg(&path)
        .status()
        .unwrap();
    assert!(status.success());
}

#[test]
fn only_a_refused_write_blames_full_disk_access() {
    use crate::{
        common::model::{Failure, Phase},
        platform::darwin::refused_for_full_disk_access,
    };

    let failure = |tail: &[&str]| Failure {
        phase: Phase::Activating,
        message: "activate failed with exit status: 1".into(),
        tail: tail.iter().map(|line| (*line).to_owned()).collect(),
    };
    assert!(refused_for_full_disk_access(&failure(&[
        "setting up /etc...",
        "ln: failed to create symbolic link '/etc/pam.d/sudo_local': Operation not permitted",
    ])));
    assert!(!refused_for_full_disk_access(&failure(&[
        "error: Unexpected files in /etc, aborting activation",
        "  /etc/nix/nix.custom.conf",
    ])));
    assert!(!refused_for_full_disk_access(&failure(&[
        "Error: An existing /opt/homebrew/Library/Taps is in the way",
    ])));
}
