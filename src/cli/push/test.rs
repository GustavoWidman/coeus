use std::{
    collections::BTreeMap,
    time::{Duration, SystemTime},
};

use gjallar::{Receiver, VapidKey};

use super::*;
use crate::{
    cli::{
        deployment::{Deployment, Following},
        dispatch::Shipment,
        exit::Exit,
        rollout::Rollout,
    },
    client::{Fleet, Offline, Tailnet, Targets},
    common::model::{
        Failure, JobId, JobState, NarHash, NixOptions, OrderId, Origin as JobOrigin, Phase,
    },
    config::{ClientSettings, HostEntry},
    testing::{
        GitFlake, Served,
        push::{entry, fake_push_service, origin, settings},
    },
    ui::{
        board::Ordering,
        test::{SHA, admit, order_board, show, summary, view},
    },
};

const SUBJECT: &str = "chore(flake): bump nixpkgs";

fn order(action: Action, source: Source) -> Order {
    Order {
        id: OrderId(0x9f3c_0a1b_2c3d_4e5f),
        source,
        action,
        options: NixOptions::default(),
        origin: JobOrigin::GitHub {
            repository: "r3dlust/nix".into(),
            workflow: "deploy".into(),
            run_id: 42,
            attempt: 1,
        },
        now: false,
    }
}

fn git() -> Source {
    Source::Git {
        rev: SHA.parse().unwrap(),
        git_ref: Some("main".into()),
    }
}

fn host(name: &str) -> Hostname {
    name.parse().unwrap()
}

fn machine(name: &str, end: End) -> Machine {
    Machine {
        host: host(name),
        end,
    }
}

fn switched(name: &str) -> Machine {
    machine(name, End::Finished(Outcome::Succeeded))
}

fn asleep(name: &str) -> Machine {
    machine(
        name,
        End::Unreachable(Unreach::Offline(Offline { last_seen: None })),
    )
}

fn failed(name: &str, phase: Phase, message: &str) -> Machine {
    machine(
        name,
        End::Finished(Outcome::Failed(Failure::new(phase, message))),
    )
}

/// what a notice says, title then body, as a lock screen shows it
fn shown(notice: &Notice) -> String {
    format!("{}\n{}", notice.title, notice.body)
}

#[test]
fn a_deploy_with_machines_asleep_counts_them_pending_not_failed() {
    let machines = [
        switched("lab"),
        switched("altar"),
        switched("oracle-1"),
        switched("oracle-2"),
        asleep("laptop-mac"),
        asleep("desktop-nixos"),
    ];
    let notice = deploy_notice(
        &order(Action::Switch, git()),
        &machines,
        Duration::from_secs(400),
        Some(SUBJECT),
    );

    assert_eq!(
        shown(&notice),
        "Deployed caf4e2b · 4 of 6\n\
         chore(flake): bump nixpkgs\n\
         4 switched in 6m40s\n\
         pending: laptop-mac, desktop-nixos (asleep)"
    );
    assert!(!notice.urgent);
    assert_eq!(notice.path, "/deployments/9f3c0a1b2c3d4e5f");
    assert_eq!(notice.tag, "deploy/switch/caf4e2b9a1d6");
}

#[test]
fn a_failure_names_its_machine_and_what_it_held_back() {
    let machines = [
        failed(
            "lab",
            Phase::Evaluating,
            "stack overflow (possible infinite recursion)\nat flake.nix:12",
        ),
        machine("altar", End::Held),
        machine("oracle-1", End::Held),
        machine("oracle-2", End::Held),
        asleep("laptop-mac"),
    ];
    let notice = deploy_notice(
        &order(Action::Switch, git()),
        &machines,
        Duration::from_secs(70),
        Some(SUBJECT),
    );

    assert_eq!(
        shown(&notice),
        "Deploy caf4e2b failed on lab\n\
         chore(flake): bump nixpkgs\n\
         lab failed evaluating: stack overflow (possible infinite recursion) · 3 held back · \
         pending: laptop-mac (asleep)"
    );
    assert!(notice.urgent);
}

#[test]
fn a_retried_run_replaces_the_notice_of_the_run_it_retries() {
    let first = deploy_notice(
        &order(Action::Switch, git()),
        &[failed(
            "lab",
            Phase::Fetching,
            "could not resolve github.com",
        )],
        Duration::from_secs(5),
        Some(SUBJECT),
    );
    let mut retry_order = order(Action::Switch, git());
    retry_order.id = OrderId(0x0123_4567_89ab_cdef);
    if let JobOrigin::GitHub { attempt, .. } = &mut retry_order.origin {
        *attempt = 2;
    }
    let retry = deploy_notice(
        &retry_order,
        &[
            "lab",
            "altar",
            "oracle-1",
            "oracle-2",
            "laptop-mac",
            "desktop-nixos",
        ]
        .map(|name| machine(name, End::Finished(Outcome::UpToDate))),
        Duration::from_secs(41),
        Some(SUBJECT),
    );

    assert_eq!(
        shown(&retry),
        "Deployed caf4e2b · 6 of 6\nchore(flake): bump nixpkgs\n6 up to date in 41s"
    );
    assert_eq!(retry.tag, first.tag);
    assert_ne!(retry.path, first.path);

    let built = deploy_notice(
        &order(Action::Build, git()),
        &[switched("lab")],
        Duration::ZERO,
        None,
    );
    assert_ne!(built.tag, first.tag, "another action is another notice");
}

#[test]
fn one_machine_reads_as_where_it_went() {
    let one = |action, outcome| {
        shown(&deploy_notice(
            &order(action, git()),
            &[machine("altar", End::Finished(outcome))],
            Duration::from_secs(123),
            None,
        ))
    };

    assert_eq!(
        one(Action::Switch, Outcome::Succeeded),
        "Deployed caf4e2b to altar\nswitched in 2m03s"
    );
    assert_eq!(
        one(Action::Boot, Outcome::Succeeded),
        "Staged caf4e2b for boot on altar\nset for boot in 2m03s"
    );
    assert_eq!(
        one(Action::Test, Outcome::UpToDate),
        "Activated caf4e2b on altar\nup to date in 2m03s"
    );
    assert_eq!(
        one(Action::Build, Outcome::Succeeded),
        "Built caf4e2b on altar\nbuilt in 2m03s"
    );
    assert_eq!(
        one(Action::Switch, Outcome::Cancelled),
        "Deploy caf4e2b failed on altar\naltar was cancelled"
    );
    assert_eq!(
        one(
            Action::Switch,
            Outcome::RolledBack(Failure::new(Phase::Verifying, "sshd did not come back"))
        ),
        "Deploy caf4e2b failed on altar\naltar rolled back after verifying: sshd did not come back"
    );
}

#[test]
fn every_verdict_mix_has_its_title_and_lines() {
    let cases: &[(&[Machine], &str, bool)] = &[
        (
            &[
                switched("lab"),
                machine("altar", End::Finished(Outcome::UpToDate)),
                switched("oracle-1"),
                machine(
                    "oracle-2",
                    End::Finished(Outcome::Superseded { by: JobId(9) }),
                ),
            ],
            "Deployed caf4e2b · 4 of 4\n2 switched, 1 up to date, 1 superseded in 1m00s",
            false,
        ),
        (
            &[
                switched("lab"),
                machine("altar", End::Rejected(Rejection::Restarting)),
            ],
            "Deploy caf4e2b refused by altar\n1 switched in 1m00s\n\
             altar refused: the daemon is restarting into a newer version, retry in a moment",
            true,
        ),
        (
            &[
                machine("altar", End::Rejected(Rejection::Restarting)),
                machine("lab", End::Finished(Outcome::Interrupted)),
                failed("oracle-1", Phase::Building, "builder failed"),
            ],
            "Deploy caf4e2b failed on lab +2\n\
             lab was interrupted · oracle-1 failed building: builder failed · \
             altar refused: the daemon is restarting into a newer version, retry in a moment",
            true,
        ),
        (
            &[
                switched("lab"),
                asleep("laptop-mac"),
                machine(
                    "oracle-1",
                    End::Unreachable(Unreach::Refused("connection refused".into())),
                ),
                machine(
                    "oracle-2",
                    End::Unreachable(Unreach::Failed("timed out".into())),
                ),
            ],
            "Deployed caf4e2b · 1 of 4\n1 switched in 1m00s\n\
             pending: laptop-mac (asleep), oracle-1, oracle-2 (unreachable)",
            false,
        ),
        (
            &[asleep("laptop-mac"), asleep("desktop-nixos")],
            "Deploy caf4e2b pending on laptop-mac +1\npending: laptop-mac, desktop-nixos (asleep)",
            false,
        ),
        (
            // an asleep canary stops the rollout, which is still no failure
            &[
                asleep("lab"),
                machine("altar", End::Held),
                machine("oracle-1", End::Held),
            ],
            "Deploy caf4e2b pending on lab\n2 held back · pending: lab (asleep)",
            false,
        ),
    ];

    for (machines, expected, urgent) in cases {
        let notice = deploy_notice(
            &order(Action::Switch, git()),
            machines,
            Duration::from_secs(60),
            None,
        );
        assert_eq!(shown(&notice), *expected);
        assert_eq!(notice.urgent, *urgent, "{expected}");
        assert_eq!(notice.tag, "deploy/switch/caf4e2b9a1d6");
    }
}

#[test]
fn a_rollback_names_its_machine_and_generation() {
    let rollback = order(Action::Switch, Source::Generation(17));
    let notice = deploy_notice(
        &rollback,
        &[switched("altar")],
        Duration::from_secs(12),
        None,
    );
    assert_eq!(
        shown(&notice),
        "Rolled altar back to generation 17\nrolled back in 12s"
    );
    assert_eq!(notice.tag, "rollback/altar/17");

    let failed = deploy_notice(
        &rollback,
        &[failed(
            "altar",
            Phase::Activating,
            "switch-to-configuration failed",
        )],
        Duration::from_secs(12),
        None,
    );
    assert_eq!(
        failed.title, "Rollback to generation 17 failed on altar",
        "{failed:?}"
    );
    assert_eq!(failed.tag, notice.tag);
}

#[test]
fn a_snapshot_is_named_by_its_label_and_tagged_by_its_hash() {
    let snapshot = Source::Snapshot {
        nar_hash: "sha256-/1lXZYH8dvmK4zp+Gm2WZkoaUVjcgL8yrEvwZ5HuFvY="
            .parse::<NarHash>()
            .unwrap(),
        label: "nix (dirty)".into(),
    };
    let notice = deploy_notice(
        &order(Action::Switch, snapshot),
        &[switched("altar")],
        Duration::from_secs(5),
        None,
    );
    assert_eq!(notice.title, "Deployed nix (dirty) to altar");
    assert_eq!(notice.tag, "deploy/switch/_1lXZYH8dvmK");
}

#[test]
fn a_deploy_no_machine_could_resolve_still_says_so() {
    let notice = unresolved_notice("main", Action::Switch);
    assert_eq!(
        shown(&notice),
        "Deploy of main reached no machine\nno machine could resolve main"
    );
    assert!(notice.urgent);
    assert_eq!(notice.path, "/");
}

#[test]
fn long_words_are_cut_to_fit_a_lock_screen() {
    let machines = [failed(
        "lab",
        Phase::Building,
        &format!("builder for '{}' failed", "x".repeat(200)),
    )];
    let long = format!("Deploy {}", "y".repeat(100));
    let notice = deploy_notice(
        &order(Action::Switch, git()),
        &machines,
        Duration::ZERO,
        Some(&long),
    );

    let lines: Vec<&str> = notice.body.lines().collect();
    assert_eq!(lines[0].chars().count(), 100);
    assert!(lines[0].ends_with('…'));
    assert_eq!(
        lines[1].chars().count(),
        "lab failed building: ".len() + 80,
        "{}",
        lines[1]
    );
    assert!(notice.title.chars().count() <= 64);

    let many: Vec<Machine> = (0..60)
        .map(|n| failed(&format!("machine-{n}"), Phase::Building, "builder failed"))
        .collect();
    let notice = deploy_notice(&order(Action::Switch, git()), &many, Duration::ZERO, None);
    assert_eq!(notice.title, "Deploy caf4e2b failed on machine-0 +59");
    assert!(notice.body.chars().count() <= 300, "{}", notice.body);
    assert!(notice.body.ends_with('…'));
}

#[test]
fn a_settled_board_gives_every_machine_its_end() {
    let mut board = order_board(&["lab", "altar", "laptop-mac", "oracle", "desktop"]);
    admit(&mut board, "lab", 12);
    show(
        &mut board,
        view(
            "lab",
            None,
            vec![summary(12, JobState::Finished(Outcome::Succeeded))],
        ),
    );
    board.order(&host("altar"), Ordering::Rejected(Rejection::Restarting));
    let offline = Unreach::Offline(Offline { last_seen: None });
    board.order(&host("laptop-mac"), Ordering::Unreachable(offline.clone()));
    board.order(&host("oracle"), Ordering::Held);
    admit(&mut board, "desktop", 3);

    assert_eq!(
        machines(&board),
        [
            switched("lab"),
            machine("altar", End::Rejected(Rejection::Restarting)),
            machine("laptop-mac", End::Unreachable(offline)),
            machine("oracle", End::Held),
        ],
        "a machine still running is left out"
    );
}

#[test]
fn the_message_is_declarative_and_opens_the_deployment() {
    let notice = deploy_notice(
        &order(Action::Switch, git()),
        &[switched("altar")],
        Duration::from_secs(123),
        Some(SUBJECT),
    );
    let at = SystemTime::UNIX_EPOCH + Duration::from_millis(1_795_038_000_000);
    let message = notice.message(&origin(), at).unwrap();

    let json: serde_json::Value = serde_json::from_slice(message.payload()).unwrap();
    assert_eq!(
        json,
        serde_json::json!({
            "web_push": 8030,
            "notification": {
                "title": "Deployed caf4e2b to altar",
                "body": "chore(flake): bump nixpkgs\nswitched in 2m03s",
                "navigate": "https://coeus.example.com/deployments/9f3c0a1b2c3d4e5f",
                "tag": "deploy/switch/caf4e2b9a1d6",
                "timestamp": 1_795_038_000_000_u64,
                "lang": "en",
                "dir": "ltr",
            }
        })
    );
}

#[test]
fn the_subject_comes_from_the_repository_at_hand() {
    let flake = GitFlake::init();
    let rev = flake.git(&["rev-parse", "HEAD"]);
    let source = Source::Git {
        rev: rev.parse().unwrap(),
        git_ref: None,
    };
    assert_eq!(subject(flake.dir.path(), &source).as_deref(), Some("first"));

    let unknown = Source::Git {
        rev: SHA.parse().unwrap(),
        git_ref: None,
    };
    assert_eq!(subject(flake.dir.path(), &unknown), None);
    assert_eq!(subject(flake.dir.path(), &Source::Generation(3)), None);
}

/// a fleet of `lab`, served by a test engine; `laptop-mac`, which tailscale
/// sees asleep before its name even resolves; and `oracle`, where nothing
/// listens
async fn fleet(served: &Served, push: Option<PushSettings>) -> Fleet {
    let closed = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let closed = closed.local_addr().unwrap().port();
    let mut settings = ClientSettings {
        connect_timeout: Duration::from_secs(5),
        push,
        ..ClientSettings::default()
    };
    for (name, address, port) in [
        ("lab", "127.0.0.1", served.endpoint.port),
        ("laptop-mac", "laptop-mac.tail1.ts.net", 45729),
        ("oracle", "127.0.0.1", closed),
    ] {
        settings.hosts.insert(
            host(name),
            HostEntry {
                address: Some(address.into()),
                port: Some(port),
                ..HostEntry::default()
            },
        );
    }

    let tailnet = Tailnet::parse(
        br#"{"BackendState":"Running",
            "Self":{"DNSName":"altar.tail1.ts.net.","TailscaleIPs":["100.64.0.1"],"Online":true},
            "Peer":{"nodekey:a":{"DNSName":"laptop-mac.tail1.ts.net.","TailscaleIPs":["100.64.0.2"],
                    "Online":false,"LastSeen":"2026-11-15T23:41:00Z"}}}"#,
    )
    .unwrap();
    let targets = Targets::select(&["@all".to_owned()], &settings).unwrap();
    Fleet::new(settings, served.key.clone(), targets).seeing(tailnet)
}

fn deployment(fleet: Fleet, following: Following, push: Option<PushSettings>) -> Deployment {
    let mut order = order(Action::Build, git());
    order.id = OrderId::random();
    Deployment {
        rollout: Rollout::plan(fleet.hosts(), &[], None, 0, false).unwrap(),
        fleet,
        shipment: Shipment::order(order),
        following,
        notify: false,
        skip_unreachable: false,
        push,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_followed_deploy_pushes_one_summary_and_a_detached_one_none() {
    let receiver = Receiver::generate().unwrap();
    let (service, seen) = fake_push_service(receiver.clone()).await;
    let dir = tempfile::tempdir().unwrap();
    let push = settings(
        dir.path(),
        &VapidKey::generate().unwrap(),
        &[
            entry("iphone", &receiver, &format!("{service}/push/iphone")),
            entry("old-laptop", &receiver, &format!("{service}/push/gone")),
        ],
    );
    let served = Served::start().await;
    let fleet = fleet(&served, Some(push.clone())).await;
    let follow = Following::Board { live: false };
    let release = served.harness.release.clone();
    let releasing = tokio::spawn(async move {
        loop {
            release.notify_one();
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    });

    let exit = deployment(fleet.clone(), follow, Some(push.clone()))
        .run()
        .await
        .unwrap();
    assert_eq!(exit, Exit::Unreached, "a push changes no exit status");

    let notifications = |seen: &BTreeMap<String, Vec<serde_json::Value>>| {
        seen.get("iphone")
            .map(|payloads| {
                payloads
                    .iter()
                    .map(|payload| payload["notification"].clone())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    };
    {
        let seen = seen.lock().unwrap();
        let notifications = notifications(&seen.payloads);
        assert_eq!(notifications.len(), 1, "{notifications:?}");
        let notification = &notifications[0];
        assert_eq!(notification["title"], "Built caf4e2b · 1 of 3");
        let body = notification["body"].as_str().unwrap();
        assert!(body.starts_with("1 built in "), "{body}");
        assert!(
            body.ends_with("\npending: laptop-mac (asleep), oracle (unreachable)"),
            "{body}"
        );
        assert_eq!(notification["tag"], "deploy/build/caf4e2b9a1d6");
        assert_eq!(
            seen.payloads["gone"].len(),
            1,
            "a gone device is still tried"
        );
    }

    // the run retried: a new order of the same commit, under the same tag
    let exit = deployment(fleet.clone(), follow, Some(push.clone()))
        .run()
        .await
        .unwrap();
    assert_eq!(exit, Exit::Unreached);
    {
        let seen = seen.lock().unwrap();
        let notifications = notifications(&seen.payloads);
        assert_eq!(notifications.len(), 2);
        assert_eq!(notifications[1]["tag"], notifications[0]["tag"]);
        assert_ne!(notifications[1]["navigate"], notifications[0]["navigate"]);
        assert!(seen.topics.iter().all(|topic| *topic == seen.topics[0]));
    }

    let exit = deployment(fleet.clone(), Following::Detach, Some(push.clone()))
        .run()
        .await
        .unwrap();
    assert_eq!(exit, Exit::Unreached);
    assert_eq!(notifications(&seen.lock().unwrap().payloads).len(), 2);

    // push files that cannot be read leave the deploy as it was
    let broken = PushSettings {
        vapid_key_file: dir.path().join("missing"),
        ..push
    };
    let exit = deployment(fleet, follow, Some(broken)).run().await.unwrap();
    assert_eq!(exit, Exit::Unreached);
    assert_eq!(notifications(&seen.lock().unwrap().payloads).len(), 2);

    releasing.abort();
    served.stop();
}

/// what the dashboard's service worker receives, pinned from the sending end:
/// the app's tests read `contract/push.json`, and a change to how a notice
/// becomes a message fails here until it is rewritten with
/// `COEUS_BLESS=1 cargo test cli::push`
#[test]
fn the_contract_holds_the_message_a_deploy_sends() {
    const CONTRACT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/contract/push.json");
    let notice = deploy_notice(
        &order(Action::Switch, git()),
        &[switched("lab"), switched("altar"), asleep("laptop-mac")],
        Duration::from_secs(400),
        Some(SUBJECT),
    );
    let at = SystemTime::UNIX_EPOCH + Duration::from_millis(1_795_038_000_000);
    let message = notice.message(&origin(), at).unwrap();
    let sent = format!("{}\n", std::str::from_utf8(message.payload()).unwrap());

    if std::env::var_os("COEUS_BLESS").is_some() {
        std::fs::write(CONTRACT, &sent).unwrap();
        return;
    }
    assert!(
        std::fs::read_to_string(CONTRACT).unwrap_or_default() == sent,
        "{CONTRACT} is not the message a deploy sends; rewrite it with `COEUS_BLESS=1 cargo test cli::push`"
    );
}
