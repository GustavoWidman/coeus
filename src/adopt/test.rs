use std::{process::Command, time::Duration};

use clap::Parser;

use super::{
    Request,
    admit::{self, Admitted, Revival},
    follow::{self, Ended},
    settings,
};
use crate::{
    cli::daemon::{DaemonCLIArgs, DaemonCommand},
    common::model::{
        Attempt, HostInfo, JobEvent, JobState, MachineView, NarHash, OrderId, Origin, Outcome,
        Phase, Placement, Platform, Source, StorePath, bootstrap_order,
    },
    config::DaemonSettings,
    job::{JobDir, JobStore},
    testing::{Harness, TOPLEVEL},
};

const NAR_HASH: &str = "sha256-Qk3+/abcdefghijklmnopqrstuvwxyz0123456789AB=";
const SOURCE: &str = "/nix/store/x0hrbv5ypmvrz3432rcg8gc9q62ipkl9-source";

fn nar_hash() -> NarHash {
    NAR_HASH.parse().unwrap()
}

fn request(attempt: u8) -> Request {
    Request {
        source: SOURCE.parse().unwrap(),
        nar_hash: nar_hash(),
        label: "altar:coeus/bootstrap/lab@a1b2c3d".into(),
        hostname: "lab".parse().unwrap(),
        order: bootstrap_order(&nar_hash(), attempt),
        install_bootloader: true,
        follow: None,
    }
}

fn host() -> HostInfo {
    HostInfo {
        hostname: "lab".parse().unwrap(),
        platform: Platform::NixOs,
        system: "x86_64-linux".into(),
        version: "test".into(),
    }
}

fn settings(state: &std::path::Path) -> DaemonSettings {
    DaemonSettings {
        flake: "git+https://example.com/nix".into(),
        state_dir: state.to_owned(),
        ..Default::default()
    }
}

/// admits try `attempt` the way `coeusd adopt` does, without starting a
/// runner
async fn adopt(store: &JobStore, state: &std::path::Path, attempt: u8) -> eyre::Result<Admitted> {
    let request = request(attempt);
    let settings = settings(state);
    let spec = |id| request.spec(id, &host(), &settings);
    admit::admit(store, &request.nar_hash, request.order, spec).await
}

fn created(admitted: Admitted) -> JobDir {
    match admitted {
        Admitted::Created(dir) => dir,
        Admitted::Found(dir) => panic!("expected a new job, found job {}", dir.id()),
    }
}

async fn released(dir: &JobDir) {
    for _ in 0..40 {
        if !dir.log().is_held().unwrap() {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("the log stayed held after its writer went");
}

/// a runner that took the job and got as far as `phase`
fn runner(dir: &JobDir, phase: Phase) -> crate::job::LogWriter {
    let mut writer = dir.log().writer().unwrap().unwrap();
    writer
        .event(JobEvent::Started {
            attempt: 1,
            pid: 0,
            version: "test".into(),
        })
        .unwrap();
    writer
        .event(JobEvent::Evaluated {
            drv: TOPLEVEL.parse().unwrap(),
            toplevel: TOPLEVEL.parse().unwrap(),
        })
        .unwrap();
    writer.event(JobEvent::Phase(phase)).unwrap();
    writer
}

#[tokio::test]
async fn admits_the_job_coeusd_would_have_written() {
    let state = tempfile::tempdir().unwrap();
    let store = JobStore::open(state.path()).unwrap();

    let dir = created(adopt(&store, state.path(), 0).await.unwrap());
    let spec = dir.spec().unwrap();

    assert_eq!(spec.order.id, bootstrap_order(&nar_hash(), 0));
    assert_eq!(
        spec.order.source,
        Source::Snapshot {
            nar_hash: nar_hash(),
            label: "altar:coeus/bootstrap/lab@a1b2c3d".into(),
        }
    );
    assert_eq!(
        spec.order.origin,
        Origin::User {
            user: "bootstrap".into(),
            host: "altar".into()
        }
    );
    assert!(spec.install_bootloader);
    assert_eq!(spec.target.sources, state.path().join("sources"));
    let guard = spec.guard.expect("the configuration's guard is on");
    assert_eq!(guard.daemon_port, 45729);
    assert!(guard.tailscale);
}

#[tokio::test]
async fn the_same_order_never_makes_a_second_job() {
    let state = tempfile::tempdir().unwrap();
    let store = JobStore::open(state.path()).unwrap();

    let first = created(adopt(&store, state.path(), 0).await.unwrap());
    let Admitted::Found(again) = adopt(&store, state.path(), 0).await.unwrap() else {
        panic!("a repeated order must find its job");
    };
    assert_eq!(again.id(), first.id());
    assert_eq!(store.list().unwrap().len(), 1);
}

#[tokio::test]
async fn a_new_try_is_admitted_only_after_the_last_one_failed() {
    let state = tempfile::tempdir().unwrap();
    let store = JobStore::open(state.path()).unwrap();
    let first = created(adopt(&store, state.path(), 0).await.unwrap());

    let early = adopt(&store, state.path(), 1).await.unwrap_err();
    assert!(early.to_string().contains("already switches"), "{early}");

    first.close(Outcome::Interrupted).unwrap();
    let skipped = adopt(&store, state.path(), 2).await.unwrap_err();
    assert!(
        skipped.to_string().contains("not the next try"),
        "{skipped}"
    );

    let second = created(adopt(&store, state.path(), 1).await.unwrap());
    assert_eq!(second.id().0, first.id().0 + 1);
}

#[tokio::test]
async fn refuses_an_order_of_another_snapshot() {
    let state = tempfile::tempdir().unwrap();
    let store = JobStore::open(state.path()).unwrap();
    let request = request(0);
    let settings = settings(state.path());

    let refused = admit::admit(&store, &request.nar_hash, OrderId(7), |id| {
        request.spec(id, &host(), &settings)
    })
    .await
    .unwrap_err();
    assert!(refused.to_string().contains("not a bootstrap order"));
    assert!(store.list().unwrap().is_empty());
}

#[tokio::test]
async fn takes_up_a_job_left_unfinished_as_coeusd_would() {
    let state = tempfile::tempdir().unwrap();
    let store = JobStore::open(state.path()).unwrap();
    let dir = created(adopt(&store, state.path(), 0).await.unwrap());
    let toplevel: StorePath = TOPLEVEL.parse().unwrap();

    // admitted, but adopt stopped before starting its runner
    assert_eq!(admit::revive(&dir, None).await.unwrap(), Revival::Spawn);

    let writer = runner(&dir, Phase::Building);
    assert_eq!(admit::revive(&dir, None).await.unwrap(), Revival::Running);

    // the runner died building: another may build again. a child another
    // test forks shares the log's open file until it execs, so the lock is
    // let go of a moment after the writer is
    drop(writer);
    released(&dir).await;
    assert_eq!(admit::revive(&dir, None).await.unwrap(), Revival::Spawn);

    // the runner died activating, and the machine runs what it built
    drop(runner(&dir, Phase::Activating));
    released(&dir).await;
    assert_eq!(
        admit::revive(&dir, Some(&toplevel)).await.unwrap(),
        Revival::Closed(Outcome::Succeeded)
    );
    assert_eq!(
        dir.summary().unwrap().unwrap().state,
        JobState::Finished(Outcome::Succeeded)
    );
    assert_eq!(admit::revive(&dir, None).await.unwrap(), Revival::Finished);
}

#[tokio::test]
async fn the_coeusd_a_switch_starts_follows_the_adopted_job_as_its_own() {
    let state = tempfile::tempdir().unwrap();
    let store = JobStore::open(state.path()).unwrap();
    let dir = created(adopt(&store, state.path(), 0).await.unwrap());
    let writer = runner(&dir, Phase::Activating);

    let harness = Harness::start(state).await;
    let view = harness
        .until(|view| {
            view.running
                .as_ref()
                .is_some_and(|job| job.summary.id == dir.id())
        })
        .await;
    assert_eq!(
        view.running.unwrap().summary.state.phase(),
        Some(Phase::Activating)
    );

    // the CLI ordering the same try through coeusd gets the adopted job
    let mut order = dir.spec().unwrap().order;
    order.origin = Origin::Follow;
    let admission = harness.engine.deploy(order).await.unwrap().unwrap();
    assert_eq!(admission.job, dir.id());
    assert_eq!(admission.placement, Placement::Duplicate);

    writer.finish(Outcome::Succeeded).unwrap();
    let finished = |view: &MachineView| {
        view.recent
            .iter()
            .any(|job| job.id == dir.id() && job.state.is_finished())
    };
    let view = harness.until(finished).await;

    let Attempt::Succeeded(job) = Attempt::of(&view.recent, &nar_hash()) else {
        panic!("history must show the switch succeeded");
    };
    assert_eq!(job.id, dir.id());
    assert!(dir.summary().unwrap().is_some());
    harness.shutdown.cancel();
}

async fn streamed(dir: &JobDir, from: u64, patience: Duration) -> (Ended, Vec<String>) {
    let mut out = Vec::new();
    let ended = follow::stream_within(dir, from, &mut out, patience)
        .await
        .unwrap();
    let lines = String::from_utf8(out)
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect();
    (ended, lines)
}

#[tokio::test]
async fn streams_the_log_with_offsets_to_resume_from() {
    let state = tempfile::tempdir().unwrap();
    let store = JobStore::open(state.path()).unwrap();
    let dir = created(adopt(&store, state.path(), 0).await.unwrap());
    let mut writer = runner(&dir, Phase::Building);
    writer.output(r#"@nix {"action":"stop","id":1}"#).unwrap();
    writer.flush().unwrap();
    writer.finish(Outcome::Succeeded).unwrap();
    dir.write_summary(&dir.view().await.unwrap().summary)
        .unwrap();

    let (ended, lines) = streamed(&dir, 0, Duration::ZERO).await;
    assert_eq!(ended, Ended::Finished);
    assert!(lines[0].starts_with("@job {"), "{lines:?}");
    assert!(lines[1].starts_with("@coeus {") && lines[1].contains("started"));
    assert!(lines[2].starts_with("@at "));
    assert_eq!(lines[7], r#"@nix {"action":"stop","id":1}"#);
    assert!(lines[9].contains("finished"), "{lines:?}");
    assert_eq!(lines.len(), 11);

    // a follower that lost the stream resumes where it says
    let at: u64 = lines[6].strip_prefix("@at ").unwrap().parse().unwrap();
    let (ended, resumed) = streamed(&dir, at, Duration::ZERO).await;
    assert_eq!(ended, Ended::Finished);
    assert_eq!(resumed[1..], lines[7..]);
}

#[tokio::test]
async fn follows_a_live_runner_until_it_finishes() {
    let state = tempfile::tempdir().unwrap();
    let store = JobStore::open(state.path()).unwrap();
    let dir = created(adopt(&store, state.path(), 0).await.unwrap());
    let writer = runner(&dir, Phase::Building);

    let following = tokio::spawn({
        let dir = dir.clone();
        async move { streamed(&dir, 0, Duration::from_secs(60)).await }
    });
    tokio::time::sleep(Duration::from_millis(300)).await;
    writer.finish(Outcome::Succeeded).unwrap();

    let (ended, lines) = tokio::time::timeout(Duration::from_secs(10), following)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(ended, Ended::Finished);
    assert!(lines[lines.len() - 2].contains("finished"), "{lines:?}");
}

#[tokio::test]
async fn gives_up_on_a_job_no_runner_holds() {
    let state = tempfile::tempdir().unwrap();
    let store = JobStore::open(state.path()).unwrap();
    let dir = created(adopt(&store, state.path(), 0).await.unwrap());
    drop(runner(&dir, Phase::Building));

    let (ended, lines) = streamed(&dir, 0, Duration::from_millis(100)).await;
    assert_eq!(ended, Ended::Unattended);
    assert!(lines.last().unwrap().starts_with("@at "));
}

#[tokio::test]
async fn a_coeusd_listening_on_any_of_its_addresses_is_found() {
    let ipv6 = tokio::net::TcpListener::bind("[::1]:0").await.unwrap();
    let port = ipv6.local_addr().unwrap().port();
    assert!(settings::listening(format!("[::]:{port}").parse().unwrap()).await);

    // a configuration listening on one address only, here standing in for
    // its tailnet address
    let named = tokio::net::TcpListener::bind("127.0.0.2:0").await;
    if let Ok(named) = named {
        let listen = named.local_addr().unwrap();
        assert!(settings::listening(listen).await);
    }

    drop(ipv6);
    assert!(!settings::listening(format!("[::]:{port}").parse().unwrap()).await);
}

#[test]
fn the_settings_are_coeusds_own() {
    let json = br#"{
        "enable": true,
        "settings": {
            "flake": "git+https://github.com/owner/nix",
            "hostname": "lab",
            "listen": "[::]:45730",
            "history": 100,
            "guard": { "enable": true, "timeout": "5m", "tailscale": false },
            "launcher": "/Library/Application Support/coeus/coeus-launcher"
        }
    }"#;
    let hostname = "lab".parse().unwrap();

    let parsed = settings::parse(json, &hostname).unwrap();
    assert_eq!(parsed.flake, "git+https://github.com/owner/nix");
    assert_eq!(parsed.listen.port(), 45730);
    assert_eq!(parsed.guard.timeout, Duration::from_secs(300));
    assert!(!parsed.guard.tailscale);
    assert_eq!(parsed.state_dir, DaemonSettings::default().state_dir);

    let disabled = settings::parse(br#"{"enable":false,"settings":null}"#, &hostname);
    assert!(
        disabled
            .unwrap_err()
            .to_string()
            .contains("does not enable")
    );
}

#[test]
fn the_module_options_render_as_coeusd_reads_them() {
    let options = r#"{
        enable = true;
        flake = "git+https://github.com/owner/nix";
        hostname = "lab";
        listen = "[::]:45729";
        sshKeyFile = null;
        history = 100;
        follow = { ref = "deployed"; interval = "15m"; };
        guard = { enable = true; timeout = "2m"; tailscale = true; };
        notify = null;
        settings = { launcher = "/opt/launcher"; guard.timeout = "5m"; state_dir = "/srv/coeus"; };
    }"#;
    let Ok(output) = Command::new("nix")
        .args(["--extra-experimental-features", "nix-command"])
        .args(["eval", "--json", "--expr"])
        .arg(format!("({}) {options}", settings::RENDER))
        .output()
    else {
        eprintln!("skipped: no nix to evaluate with");
        return;
    };
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let parsed = settings::parse(&output.stdout, &"lab".parse().unwrap()).unwrap();
    assert_eq!(parsed.guard.timeout, Duration::from_secs(300));
    assert!(parsed.guard.enable && parsed.guard.tailscale);
    assert_eq!(parsed.state_dir, std::path::Path::new("/srv/coeus"));
    assert_eq!(parsed.launcher, std::path::Path::new("/opt/launcher"));
    assert_eq!(parsed.follow.unwrap().git_ref, "deployed");
    assert_eq!(parsed.ssh_key_file, None);
}

#[test]
fn status_reads_the_configurations_state_directory_or_the_one_it_is_told() {
    let parse = |args: &[&str]| {
        DaemonCLIArgs::try_parse_from(
            ["coeusd", "adopt", "--nar-hash", NAR_HASH]
                .iter()
                .chain(args),
        )
    };
    let state_dir = |args: &[&str]| match parse(args).unwrap().command {
        Some(DaemonCommand::Adopt(adopt)) => adopt.state_dir,
        command => panic!("not adopt: {command:?}"),
    };

    assert_eq!(state_dir(&["--status"]), None);
    assert_eq!(
        state_dir(&["--status", "--state-dir", "/srv/coeus"]),
        Some("/srv/coeus".into())
    );
    let configured = ["--status", "--source", SOURCE, "--hostname", "lab"];
    assert!(parse(&configured).is_ok());
    assert!(
        parse(&["--status", "--source", SOURCE]).is_err(),
        "the configuration is named by both"
    );
    assert_eq!(
        super::default_state_dir(),
        DaemonSettings::default().state_dir
    );

    let adopting = [
        "--source",
        SOURCE,
        "--label",
        "x",
        "--hostname",
        "lab",
        "--order",
        "0000000000000001",
    ];
    assert!(parse(&adopting).is_ok());
    let moved: Vec<&str> = adopting
        .iter()
        .copied()
        .chain(["--state-dir", "/srv"])
        .collect();
    assert!(parse(&moved).is_err(), "adopting uses the configuration's");
}
