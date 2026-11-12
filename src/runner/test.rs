use std::{ffi::OsString, os::unix::fs::symlink, path::Path, time::Duration};

use tokio::process::Command;

use super::{
    activation::{Activation, Step, is_up_to_date},
    feed::Feed,
    guard::{Health, is_running},
    job::History,
    nix::{self, Call, Nix},
    process::{Process, Stdout},
    *,
};
use crate::{
    common::model::{
        Action, JobEvent, JobId, JobState, NixOptions, Order, OrderId, Origin, Outcome, Phase,
        Platform, Source, StorePath, Timestamp,
    },
    job::{Guard, JobDir, JobSpec, JobStore, LogItem, StopRequest, Target},
    platform::SystemRoot,
};

const SHA: &str = "caf4e2b9a1d6c3e8f0b2a4c6d8e0f1a3b5c7d9e1";
const NAR_HASH: &str = "sha256-Qk3+/abcdefghijklmnopqrstuvwxyz0123456789AB=";

fn store(name: &str) -> StorePath {
    format!("/nix/store/{}-{name}", "a".repeat(32))
        .parse()
        .unwrap()
}

fn target(platform: Platform) -> Target {
    Target {
        hostname: "altar".parse().unwrap(),
        platform,
        flake: "git+https://github.com/owner/nix".into(),
        sources: "/var/lib/coeus/sources".into(),
    }
}

fn spec(source: Source, action: Action) -> JobSpec {
    JobSpec {
        id: JobId(42),
        order: Order {
            id: OrderId(7),
            source,
            action,
            options: NixOptions::default(),
            origin: Origin::Follow,
            now: false,
        },
        ordered_at: Timestamp(1_000),
        target: target(Platform::current()),
        ssh_key_file: None,
        guard: None,
        install_bootloader: false,
    }
}

fn git(git_ref: Option<&str>) -> Source {
    Source::Git {
        rev: SHA.parse().unwrap(),
        git_ref: git_ref.map(str::to_owned),
    }
}

fn strings(args: Vec<OsString>) -> Vec<String> {
    args.into_iter()
        .map(|arg| arg.into_string().unwrap())
        .collect()
}

#[test]
fn git_sources_pin_the_commit_and_keep_the_ref() {
    let target = target(Platform::Darwin);
    assert_eq!(
        target.recipe(&git(Some("main"))),
        Recipe::Flake(format!(
            "git+https://github.com/owner/nix?rev={SHA}&ref=main"
        ))
    );
    assert_eq!(
        target.recipe(&git(None)),
        Recipe::Flake(format!("git+https://github.com/owner/nix?rev={SHA}"))
    );

    let submodules = Target {
        flake: "git+https://github.com/owner/nix?submodules=1".into(),
        ..target
    };
    assert_eq!(
        submodules.recipe(&git(None)),
        Recipe::Flake(format!(
            "git+https://github.com/owner/nix?submodules=1&rev={SHA}"
        ))
    );
}

#[test]
fn snapshots_build_from_their_unpacked_directory_and_hash() {
    let source = Source::Snapshot {
        nar_hash: NAR_HASH.to_owned().try_into().unwrap(),
        label: "~/nix".into(),
    };

    assert_eq!(
        target(Platform::NixOs).recipe(&source),
        Recipe::Flake(
            "path:/var/lib/coeus/sources/sha256-Qk3-_abcdefghijklmnopqrstuvwxyz0123456789AB=\
             ?narHash=sha256-Qk3%2B%2Fabcdefghijklmnopqrstuvwxyz0123456789AB%3D"
                .into()
        )
    );
    assert_eq!(
        target(Platform::NixOs).recipe(&Source::Generation(9)),
        Recipe::Generation(9)
    );
}

#[test]
fn the_installable_is_the_hosts_toplevel() {
    assert_eq!(
        target(Platform::Darwin).toplevel("path:/src"),
        "path:/src#darwinConfigurations.altar.config.system.build.toplevel"
    );

    let dotted = Target {
        hostname: "oracle.1".parse().unwrap(),
        ..target(Platform::NixOs)
    };
    assert_eq!(
        dotted.toplevel("path:/src"),
        "path:/src#nixosConfigurations.\"oracle.1\".config.system.build.toplevel"
    );
}

#[test]
fn every_nix_call_logs_internal_json_and_never_writes_a_lock_file() {
    let nix = Nix::new(&spec(git(None), Action::Build));
    let drv = store("system.drv");
    let link = Path::new("/var/lib/coeus/jobs/000042/result");

    let evaluate = strings(nix.args(&Call::Evaluate("flake#x")));
    assert_eq!(evaluate[..4], ["build", "--dry-run", "--json", "flake#x"]);
    assert_eq!(
        evaluate[4..],
        [
            "--log-format",
            "internal-json",
            "-v",
            "--extra-experimental-features",
            "nix-command flakes pipe-operators",
            "--option",
            "accept-flake-config",
            "true",
            "--no-write-lock-file",
        ]
    );

    let build = strings(nix.args(&Call::Build { drv: &drv, link }));
    assert_eq!(
        build[..5],
        [
            "build".to_owned(),
            format!("{drv}^*"),
            "--out-link".to_owned(),
            link.display().to_string(),
            "--print-out-paths".to_owned(),
        ]
    );
    assert!(build.contains(&"internal-json".to_owned()));
    assert!(!build.contains(&"--no-write-lock-file".to_owned()));
}

#[test]
fn order_options_turn_nix_knobs() {
    let mut spec = spec(git(None), Action::Build);
    spec.order.options = NixOptions {
        no_eval_cache: true,
        clean_substituters: true,
    };
    spec.ssh_key_file = Some("/run/agenix/deploy-key".into());

    let nix = Nix::new(&spec);
    let args = strings(nix.args(&Call::Metadata("flake")));
    let options: Vec<&[String]> = args
        .windows(3)
        .filter(|window| window[0] == "--option")
        .collect();
    let named = |name: &str| {
        options
            .iter()
            .find(|option| option[1] == name)
            .map(|option| option[2].as_str())
    };

    assert_eq!(named("eval-cache"), Some("false"));
    assert_eq!(named("substituters"), Some(CLEAN_SUBSTITUTERS));
    assert_eq!(named("extra-substituters"), Some(""));
    assert_eq!(
        named("trusted-public-keys"),
        Some(CLEAN_TRUSTED_PUBLIC_KEYS)
    );
    assert_eq!(named("extra-trusted-public-keys"), Some(""));

    let command = nix.command(&Call::Metadata("flake"));
    let env: Vec<_> = command.as_std().get_envs().collect();
    assert!(env.contains(&("NIX_REMOTE".as_ref(), Some("daemon".as_ref()))));
    assert!(env.contains(&(
        "GIT_SSH_COMMAND".as_ref(),
        Some(
            "ssh -i /run/agenix/deploy-key -o IdentitiesOnly=yes -o StrictHostKeyChecking=accept-new"
                .as_ref()
        )
    )));
}

#[test]
fn reads_what_nix_answers() {
    let drv = store("darwin-system.drv");
    let out = store("darwin-system");
    let dry_run = format!(r#"[{{"drvPath":"{drv}","outputs":{{"out":"{out}"}}}}]"#);
    assert_eq!(
        nix::evaluation(dry_run.as_bytes()).unwrap(),
        (drv.clone(), out.clone())
    );
    assert!(nix::evaluation(b"[]").is_err());

    let keyed = format!(r#"{{"{out}":{{"deriver":"{drv}","narSize":1}}}}"#);
    let listed = format!(r#"[{{"path":"{out}","deriver":"{drv}"}}]"#);
    assert_eq!(nix::deriver(keyed.as_bytes()), Some(drv.clone()));
    assert_eq!(nix::deriver(listed.as_bytes()), Some(drv));
    assert_eq!(
        nix::deriver(format!(r#"{{"{out}":{{"deriver":null}}}}"#).as_bytes()),
        None
    );
}

#[test]
fn only_a_machine_already_there_is_up_to_date() {
    let new = store("system-new");
    let old = store("system-old");
    let up_to_date = |action, current: &StorePath, profile: &StorePath| {
        is_up_to_date(action, &new, Some(current), Some(profile))
    };

    assert!(up_to_date(Action::Switch, &new, &new));
    assert!(up_to_date(Action::Boot, &new, &new));
    assert!(up_to_date(Action::Test, &new, &new));
    assert!(
        !up_to_date(Action::Build, &new, &new),
        "a build always builds"
    );

    assert!(
        !up_to_date(Action::Switch, &old, &new),
        "running something else"
    );
    assert!(
        !up_to_date(Action::Switch, &new, &old),
        "the profile points elsewhere"
    );
    assert!(!up_to_date(Action::Boot, &old, &new));
    assert!(
        !is_up_to_date(Action::Switch, &new, None, None),
        "a fresh machine"
    );
}

#[test]
fn nixos_sets_the_profile_then_switches() {
    let toplevel = store("nixos-system-lab");
    let activation = |action, generation| Activation {
        platform: Platform::NixOs,
        action,
        toplevel: toplevel.clone(),
        generation,
        install_bootloader: false,
    };
    let commands = |activation: Activation| -> Vec<String> {
        activation
            .steps()
            .iter()
            .map(|step| {
                [step.program.clone()]
                    .into_iter()
                    .chain(step.args.clone())
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .collect()
    };

    assert_eq!(
        commands(activation(Action::Switch, None)),
        [
            format!("nix-env -p /nix/var/nix/profiles/system --set {toplevel}"),
            format!("{toplevel}/bin/switch-to-configuration switch"),
        ]
    );
    assert_eq!(
        commands(activation(Action::Boot, Some(9))),
        [
            "nix-env -p /nix/var/nix/profiles/system --switch-generation 9".to_owned(),
            format!("{toplevel}/bin/switch-to-configuration boot"),
        ]
    );
    assert_eq!(
        commands(activation(Action::Test, None)),
        [format!("{toplevel}/bin/switch-to-configuration test")]
    );

    let switch = activation(Action::Switch, None).steps().pop().unwrap();
    assert_eq!(switch.env, [("NIXOS_INSTALL_BOOTLOADER", None)]);

    let first = Activation {
        install_bootloader: true,
        ..activation(Action::Switch, None)
    };
    let switch = first.steps().pop().unwrap();
    assert_eq!(
        switch.env,
        [("NIXOS_INSTALL_BOOTLOADER", Some("1".to_owned()))]
    );
}

#[test]
fn specs_written_before_the_boot_loader_flag_still_parse() {
    let mut json = serde_json::to_value(spec(git(None), Action::Switch)).unwrap();
    json.as_object_mut().unwrap().remove("install_bootloader");

    let spec: JobSpec = serde_json::from_value(json).unwrap();
    assert!(!spec.install_bootloader);
}

#[test]
fn darwin_activates_as_root_with_the_system_tools() {
    let toplevel = store("darwin-system");
    let steps = Activation {
        platform: Platform::Darwin,
        action: Action::Switch,
        toplevel: toplevel.clone(),
        generation: None,
        install_bootloader: false,
    }
    .steps();

    assert_eq!(
        steps[0].args,
        [
            "-p",
            "/nix/var/nix/profiles/system",
            "--set",
            toplevel.as_str()
        ]
    );
    let Step { program, args, env } = &steps[1];
    assert_eq!(program, &format!("{toplevel}/activate"));
    assert!(args.is_empty());

    let var = |name| {
        env.iter()
            .find(|(key, _)| *key == name)
            .and_then(|(_, value)| value.clone())
    };
    assert_eq!(var("HOME").as_deref(), Some("/var/root"));
    assert_eq!(var("NIX_REMOTE").as_deref(), Some("daemon"));
    let path = var("PATH").unwrap();
    for dir in ["/usr/bin", "/bin", "/usr/sbin", "/sbin"] {
        assert!(
            path.split(':').any(|entry| entry == dir),
            "{dir} missing from {path}"
        );
    }
}

#[test]
fn undoing_returns_to_the_profile_generation_or_the_running_system() {
    let root = tempfile::tempdir().unwrap();
    let profiles = root.path().join("nix/var/nix/profiles");
    std::fs::create_dir_all(&profiles).unwrap();
    std::fs::create_dir_all(root.path().join("run")).unwrap();
    symlink(store("system-9").as_str(), profiles.join("system-9-link")).unwrap();
    symlink("system-9-link", profiles.join("system")).unwrap();
    symlink(
        store("system-8").as_str(),
        root.path().join("run/current-system"),
    )
    .unwrap();
    let system = SystemRoot::at(root.path());

    let switch = Activation::undo(Platform::NixOs, Action::Switch, false, &system).unwrap();
    assert_eq!(switch.generation, Some(9));
    assert_eq!(switch.toplevel, store("system-9"));
    assert_eq!(switch.action, Action::Switch);
    assert!(!switch.install_bootloader);

    let test = Activation::undo(Platform::NixOs, Action::Test, false, &system).unwrap();
    assert_eq!(test.generation, None);
    assert_eq!(test.toplevel, store("system-8"));
    assert_eq!(test.action, Action::Test);

    let fresh = SystemRoot::at(&root.path().join("nowhere"));
    assert_eq!(
        Activation::undo(Platform::NixOs, Action::Switch, false, &fresh),
        None
    );
}

#[test]
fn going_back_from_a_switch_that_installed_the_boot_loader_installs_it_again() {
    let root = tempfile::tempdir().unwrap();
    let profiles = root.path().join("nix/var/nix/profiles");
    std::fs::create_dir_all(&profiles).unwrap();
    symlink(store("system-9").as_str(), profiles.join("system-9-link")).unwrap();
    symlink("system-9-link", profiles.join("system")).unwrap();
    let system = SystemRoot::at(root.path());

    let undo = Activation::undo(Platform::NixOs, Action::Switch, true, &system).unwrap();
    let activate = undo.steps().pop().unwrap();
    assert_eq!(
        activate.env,
        [("NIXOS_INSTALL_BOOTLOADER", Some("1".to_owned()))]
    );
}

#[test]
fn the_guard_wants_coeusd_and_a_running_tailscale() {
    let healthy = |daemon, tailscale| Health { daemon, tailscale }.is_healthy();
    assert!(healthy(true, None));
    assert!(healthy(true, Some(true)));
    assert!(!healthy(true, Some(false)));
    assert!(!healthy(false, None));
    assert_eq!(
        Health {
            daemon: false,
            tailscale: Some(false)
        }
        .problem(),
        "coeusd does not answer and tailscale is not running"
    );

    assert!(is_running(
        br#"{"Version":"1.90","BackendState":"Running","Self":{}}"#
    ));
    assert!(!is_running(br#"{"BackendState":"Stopped"}"#));
    assert!(!is_running(b"not json"));
}

#[tokio::test]
async fn the_guard_passes_once_coeusd_answers() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let guard = |daemon_port| Guard {
        timeout: Duration::from_secs(1),
        daemon_port,
        tailscale: false,
    };

    assert_eq!(guard(port).verify().await, Ok(()));
    drop(listener);
    assert_eq!(
        guard(port).verify().await,
        Err("coeusd does not answer 1s after activating".to_owned())
    );
}

fn job(spec: &JobSpec) -> (tempfile::TempDir, JobDir) {
    let state = tempfile::tempdir().unwrap();
    let dir = JobStore::open(state.path()).unwrap().create(spec).unwrap();
    (state, dir)
}

async fn events(dir: &JobDir) -> Vec<JobEvent> {
    let mut reader = dir.log().reader(0).await.unwrap();
    let mut events = Vec::new();
    while let Some(stamped) = reader.next_event().await.unwrap() {
        events.push(stamped.event);
    }
    events
}

#[tokio::test]
async fn a_job_whose_runners_kept_dying_fails() {
    let (_state, dir) = job(&spec(git(None), Action::Build));
    {
        let mut writer = dir.log().writer().unwrap().unwrap();
        for attempt in 1..=3 {
            writer
                .event(JobEvent::Started {
                    attempt,
                    pid: 1,
                    version: "0.0.0".into(),
                })
                .unwrap();
            writer.event(JobEvent::Phase(Phase::Building)).unwrap();
        }
    }

    run(dir.path()).await.unwrap();

    let summary = dir
        .summary()
        .unwrap()
        .expect("a finished job has a summary");
    assert_eq!(summary.attempt, 4);
    let JobState::Finished(Outcome::Failed(failure)) = summary.state else {
        panic!("expected a failure, got {:?}", summary.state);
    };
    assert_eq!(
        failure.message,
        "the daemon restarted too many times during this job"
    );
}

#[tokio::test]
async fn a_stop_request_left_before_the_runner_started_ends_the_job() {
    let (_state, dir) = job(&spec(git(None), Action::Build));
    dir.request_stop(StopRequest::Superseded { by: JobId(43) })
        .unwrap();

    run(dir.path()).await.unwrap();

    let events = events(&dir).await;
    assert!(matches!(events[0], JobEvent::Started { attempt: 1, .. }));
    assert_eq!(
        events[1],
        JobEvent::Finished(Outcome::Superseded { by: JobId(43) })
    );
    assert_eq!(
        dir.summary().unwrap().unwrap().state,
        JobState::Finished(Outcome::Superseded { by: JobId(43) })
    );
}

#[tokio::test]
async fn a_second_runner_leaves_the_job_alone() {
    let (_state, dir) = job(&spec(git(None), Action::Build));
    let writer = dir.log().writer().unwrap().unwrap();

    run(dir.path()).await.unwrap();
    assert!(events(&dir).await.is_empty(), "a held log gets no events");

    writer.finish(Outcome::Cancelled).unwrap();
    run(dir.path()).await.unwrap();
    assert_eq!(events(&dir).await, [JobEvent::Finished(Outcome::Cancelled)]);
    assert_eq!(
        History::of(&dir.log()).await.unwrap(),
        History {
            started: 0,
            finished: true
        }
    );
}

#[tokio::test]
async fn activation_output_reaches_the_log_in_order() {
    let (_state, dir) = job(&spec(git(None), Action::Build));
    let mut writer = dir.log().writer().unwrap().unwrap();

    let mut command = Command::new("/bin/sh");
    command.args([
        "-c",
        r#"echo "setting up /etc..."; echo "warning: something" >&2; echo '@nix {"action":"msg","level":0,"msg":"error: boom"}' >&2; exit 3"#,
    ]);
    let mut feed = Feed::new(&mut writer);
    let exit = Process::spawn(command)
        .unwrap()
        .finish(&mut feed, Stdout::Log)
        .await
        .unwrap();

    assert_eq!(exit.status.code(), Some(3));
    assert_eq!(feed.fold().last_error(), Some("boom"));
    drop(feed);
    writer.flush().unwrap();

    let Some(LogItem::Nix(lines)) = dir
        .log()
        .reader(0)
        .await
        .unwrap()
        .next(usize::MAX)
        .await
        .unwrap()
    else {
        panic!("expected output lines");
    };
    let text = String::from_utf8(lines).unwrap();
    assert!(text.contains(r#""msg":"setting up /etc...""#));
    assert!(text.contains(r#""msg":"warning: something""#));
    assert!(text.contains(r#"@nix {"action":"msg","level":0,"msg":"error: boom"}"#));
}

#[tokio::test]
async fn stopping_a_process_kills_everything_it_started() {
    let (_state, dir) = job(&spec(git(None), Action::Build));
    let mut writer = dir.log().writer().unwrap().unwrap();
    let pids = dir.path().join("pids");

    let mut command = Command::new("/bin/sh");
    command.arg("-c").arg(format!(
        "sleep 60 & echo $! > {0}.tmp; echo $$ >> {0}.tmp; mv {0}.tmp {0}; wait",
        pids.display()
    ));
    let process = Process::spawn(command).unwrap();

    let mut feed = Feed::new(&mut writer);
    let running = process.finish(&mut feed, Stdout::Capture);
    let stopped = tokio::time::timeout(Duration::from_millis(500), running).await;
    assert!(stopped.is_err(), "the shell waits on its sleep");

    let pids: Vec<i32> = std::fs::read_to_string(&pids)
        .unwrap()
        .lines()
        .map(|pid| pid.parse().unwrap())
        .collect();
    tokio::time::sleep(Duration::from_millis(100)).await;

    for pid in pids {
        let alive = std::process::Command::new("ps")
            .args(["-o", "stat=", "-p", &pid.to_string()])
            .output()
            .unwrap();
        let stat = String::from_utf8_lossy(&alive.stdout);
        assert!(
            stat.trim().is_empty() || stat.starts_with('Z'),
            "{pid} still runs: {stat}"
        );
    }
}

#[tokio::test]
async fn a_process_is_done_when_it_exits_even_if_its_children_hold_the_output() {
    let (_state, dir) = job(&spec(git(None), Action::Build));
    let mut writer = dir.log().writer().unwrap().unwrap();

    let mut command = Command::new("/bin/sh");
    command.args(["-c", "sleep 30 & echo started"]);
    let mut feed = Feed::new(&mut writer);
    let exit = tokio::time::timeout(
        Duration::from_secs(5),
        Process::spawn(command)
            .unwrap()
            .finish(&mut feed, Stdout::Capture),
    )
    .await
    .expect("finished while the sleep holds stdout")
    .unwrap();

    assert!(exit.status.success());
    assert_eq!(exit.stdout, b"started\n");
}
