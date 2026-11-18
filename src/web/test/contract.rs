//! what the browser reads, pinned from both ends: `contract/web.json` holds
//! an example of every message as the server serializes it, and
//! `web/src/lib/wire/types.ts` declares the types the browser parses them
//! into, generated from src/web/view.rs. a change to the view fails here
//! until both files are rewritten with `COEUS_BLESS=1 cargo test web::`.

use std::collections::BTreeSet;

use serde_json::Value;

use super::{
    NOW, ORDERED, RELEASE, SHA, card, catch_up, following, git, host, job, machine, release,
    running,
    session::Browser,
    signin, succeeded,
    typescript::{literals, names, typescript},
};
use crate::{
    common::model::{
        Action, AttachFrame, ClosureDiff, DaemonState, Failure, JobEvent, JobId, JobState,
        JobSummary, Origin, Outcome, PackageChange, Phase, Platform, Progress, Source, Stamped,
        Tally, Timestamp,
    },
    web::{derive, signin::SignIn, transcript::Transcript, view::*},
};

const CONTRACT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/contract/web.json");
const TYPES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/web/src/lib/wire/types.ts");

const DAY: i64 = 24 * 60 * 60 * 1000;
const TOPLEVEL: &str = "/nix/store/h0myp2wfggr0x8pfacciv8sfhxyb845d-nixos-system-lab-26.11";

/// the file at `path` holds `generated`, or is rewritten to when blessing
fn pinned(path: &str, generated: &str) {
    if std::env::var_os("COEUS_BLESS").is_some() {
        std::fs::create_dir_all(std::path::Path::new(path).parent().unwrap()).unwrap();
        std::fs::write(path, generated).unwrap();
        return;
    }
    let pinned = std::fs::read_to_string(path).unwrap_or_default();
    assert!(
        pinned == generated,
        "{path} is not what src/web/view.rs generates; rewrite it with `COEUS_BLESS=1 cargo test web::`"
    );
}

#[test]
fn the_contract_holds_what_the_server_sends() {
    pinned(CONTRACT, &contract());
}

#[test]
fn the_typescript_types_are_the_rust_ones() {
    pinned(TYPES, &typescript());
}

/// every tag and every unit variant the types declare appears in an example
#[test]
fn the_contract_shows_every_kind_the_types_declare() {
    let mut strings = BTreeSet::new();
    collect(&serde_json::from_str(&contract()).unwrap(), &mut strings);

    let missing: Vec<String> = literals(&typescript())
        .into_iter()
        .filter(|literal| !strings.contains(literal))
        .collect();

    assert!(missing.is_empty(), "the contract shows no {missing:?}");
}

#[test]
fn every_type_the_typescript_names_is_declared() {
    let types = typescript();
    let declared: BTreeSet<&str> = types
        .lines()
        .filter_map(|line| line.strip_prefix("export type "))
        .filter_map(|line| line.split(' ').next())
        .collect();

    let undeclared: BTreeSet<String> = names(&types)
        .into_iter()
        .filter(|name| name != "Array" && !declared.contains(name.as_str()))
        .collect();

    assert!(undeclared.is_empty(), "undeclared: {undeclared:?}");
}

/// the fleet the contract shows: a release rolling out in two waves, a
/// machine asleep through it, two catching up, and two never reached; with
/// the progress of the release's running build
fn fleet() -> (Vec<MachineCard>, LiveProgress) {
    let earlier = |id, state| job(id, 0x5000 + id, -2 * DAY, state);
    let failure = |phase, message: &str| Failure::new(phase, message);

    let mut lab = following(
        machine("lab", vec![release(62, 0xa1, 0, running())]),
        "deployed",
        RELEASE,
    );
    lab.system.current = Some(TOPLEVEL.parse().unwrap());
    lab.system.deployed = Some(git(SHA, "main"));
    lab.system.reboot_required = true;
    let building = lab.running.as_mut().unwrap();
    building.summary.toplevel = Some(TOPLEVEL.parse().unwrap());
    building.phase_started_at = Some(Timestamp(ORDERED.0 + 90_000));
    building.changes = Some(14);
    building.progress = Progress {
        builds: Tally {
            done: 186,
            expected: 388,
            running: 2,
            failed: 0,
        },
        downloads: Tally {
            done: 12,
            expected: 40,
            running: 1,
            failed: 0,
        },
        downloaded_bytes: 1_288_490_189,
        expected_bytes: 3_650_722_201,
        active: vec!["firefox-121.0".into(), "linux-6.12.4".into()],
    };
    lab.next = Some(JobSummary {
        source: Source::Snapshot {
            nar_hash: "sha256-47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU="
                .parse()
                .unwrap(),
            label: "altar:~/nix".into(),
        },
        action: Action::Boot,
        ..job(63, 0xb2, -60_000, JobState::Queued)
    });
    lab.recent = vec![
        JobSummary {
            source: Source::Generation(40),
            action: Action::Build,
            ..earlier(61, JobState::Finished(Outcome::UpToDate))
        },
        JobSummary {
            action: Action::Test,
            origin: Origin::GitHub {
                repository: "r3dlust/nix".into(),
                workflow: "deploy".into(),
                run_id: 19_458_201_337,
                attempt: 1,
            },
            ..earlier(
                60,
                JobState::Finished(Outcome::Superseded { by: JobId(61) }),
            )
        },
        earlier(
            59,
            JobState::Finished(Outcome::RolledBack(failure(
                Phase::Verifying,
                "sshd.service did not come back",
            ))),
        ),
        earlier(
            58,
            JobState::Finished(Outcome::Failed(failure(
                Phase::Evaluating,
                "attribute 'zfs' missing",
            ))),
        ),
        earlier(57, JobState::Finished(Outcome::Cancelled)),
        earlier(56, JobState::Finished(Outcome::Interrupted)),
    ];

    let nova = machine(
        "nova",
        vec![release(
            20,
            0xa1,
            800,
            JobState::Finished(Outcome::Failed(failure(
                Phase::Fetching,
                "could not reach the forge",
            ))),
        )],
    );
    let mut altar = machine("altar", vec![release(30, 0xa1, 6_000, succeeded())]);
    altar.host.platform = Platform::Darwin;
    altar.host.system = "aarch64-darwin".into();
    let activating = JobState::Running {
        phase: Phase::Activating,
    };
    let mut oracle = machine("oracle", vec![release(40, 0xa1, 6_500, activating)]);
    oracle.daemon = DaemonState::Restarting;

    let mut laptop = following(machine("laptop", vec![]), "deployed", SHA);
    laptop.system.deployed = Some(git(SHA, "deployed"));
    let preparing = JobState::Running {
        phase: Phase::Preparing,
    };
    let mut tower = following(
        machine("tower", vec![catch_up(50, "deployed", preparing)]),
        "deployed",
        RELEASE,
    );
    tower.system.pending_boot = true;
    let mini = following(
        machine("mini", vec![catch_up(51, "deployed", succeeded())]),
        "deployed",
        RELEASE,
    );

    let mut cards: Vec<MachineCard> = [&lab, &nova, &altar, &oracle, &laptop, &tower, &mini]
        .into_iter()
        .map(card)
        .collect();
    cards[4].link = LinkView::Unreachable {
        since: Timestamp(ORDERED.0 - DAY),
        reason: UnreachView::Offline {
            last_seen: Some(Timestamp(ORDERED.0 - DAY - 60_000)),
        },
    };
    cards.push(MachineCard {
        host: host("spare"),
        link: LinkView::Connecting,
        state: None,
    });
    cards.push(MachineCard {
        host: host("relay"),
        link: LinkView::Unreachable {
            since: ORDERED,
            reason: UnreachView::Refused {
                detail: "the handshake failed: another fleet key, or not coeusd".into(),
            },
        },
        state: None,
    });
    (cards, derive::progress(&lab).unwrap())
}

/// the log of a job that fetched, built and succeeded
fn log() -> Vec<LogLine> {
    let at = |offset| Timestamp(ORDERED.0 + offset);
    let event = |offset, event| {
        AttachFrame::Event(Stamped {
            at: at(offset),
            event,
        })
    };
    let nix = |lines: &[&str]| AttachFrame::Nix(lines.join("\n").into_bytes());

    Transcript::default().lines(&[
        event(
            1_000,
            JobEvent::Started {
                attempt: 1,
                pid: 4242,
                version: "0.6.0".into(),
            },
        ),
        event(1_200, JobEvent::Phase(Phase::Fetching)),
        nix(&[
            r#"@nix {"action":"start","id":7,"level":3,"type":100,"text":"copying path '/nix/store/fv6rgzamfwj334kf1xv8ms3y6x8nvirn-hello-2.12.3' from 'https://cache.nixos.org'","fields":[]}"#,
        ]),
        event(30_000, JobEvent::Phase(Phase::Building)),
        nix(&[
            r#"@nix {"action":"start","id":9,"level":3,"type":105,"text":"building '/nix/store/9rfvhi1nqszqmny102jqxp2vmb6n0mk7-zfs-kernel-2.3.4.drv'","fields":["/nix/store/9rfvhi1nqszqmny102jqxp2vmb6n0mk7-zfs-kernel-2.3.4.drv","",1,1]}"#,
            r#"@nix {"action":"result","id":9,"type":101,"fields":["CC [M]  module/zfs/arc.o"]}"#,
            r#"@nix {"action":"msg","level":0,"msg":"\u001b[31;1merror:\u001b[0m cannot connect to 'ssh-ng://builder'"}"#,
        ]),
        event(
            200_000,
            JobEvent::Note("the build fell back to building locally".into()),
        ),
        event(241_000, JobEvent::Finished(Outcome::Succeeded)),
    ])
}

fn diff() -> ClosureDiff {
    let change = |name: &str, before: &[&str], after: &[&str], size_delta| PackageChange {
        name: name.into(),
        before: before.iter().map(|&version| version.into()).collect(),
        after: after.iter().map(|&version| version.into()).collect(),
        size_delta,
    };
    ClosureDiff {
        changes: vec![
            change("firefox", &["120.0"], &["121.0"], 1_258_291),
            change("zfs-kernel", &[], &["2.3.4"], 9_437_184),
            change("python3.11", &["3.11.9"], &[], -104_857_600),
        ],
    }
}

/// one example of every message, as the server serializes it
fn contract() -> String {
    let (cards, progress) = fleet();
    let running = cards[0].state.as_ref().unwrap().running.clone().unwrap();
    let mut deployments = derive::deployments(&cards, NOW);
    // altar finished, so its closure diff was read
    deployments[0].rows[2].diff = Some(DiffSummary::from(&diff()));

    let fleet = vec![
        FleetEvent::Snapshot(Snapshot {
            schema: SCHEMA,
            build: "0.6.0-4f2a9c1".into(),
            served_by: host("lab"),
            viewer: Viewer {
                login: "r3dlust@github".into(),
                device: "iphone".into(),
                expires_at: Timestamp(ORDERED.0 + 7 * DAY),
            },
            machines: cards.clone(),
            progress: vec![progress.clone()],
            deployments: deployments.clone(),
        }),
        FleetEvent::Machine {
            machine: Box::new(MachineCard {
                host: host("relay"),
                link: LinkView::Unreachable {
                    since: NOW,
                    reason: UnreachView::Failed {
                        detail: "connection reset by peer".into(),
                    },
                },
                state: None,
            }),
        },
        FleetEvent::Progress { progress },
        FleetEvent::ProgressCleared { host: host("lab") },
        FleetEvent::Deployment {
            deployment: deployments[0].clone(),
        },
        FleetEvent::DeploymentRemoved {
            order: "0000000000005038".into(),
        },
        FleetEvent::Expired,
    ];
    let log = vec![
        LogEvent::Started {
            job: Box::new(running),
        },
        LogEvent::Lines {
            lines: log(),
            end: 18_342,
        },
        LogEvent::Ended {
            outcome: OutcomeView::Succeeded,
        },
        LogEvent::Expired,
    ];

    #[derive(serde::Serialize)]
    struct Auth {
        hello: Hello,
        signin: SignIn,
        signed_in: SignedIn,
    }
    // RFC 6979 signs deterministically, so a fixed key signs the same bytes
    let browser = Browser::seeded(7);
    let auth = Auth {
        hello: Hello {
            signin: vec![
                "https://lab.tail0000.ts.net:8443".into(),
                "https://oracle.tail0000.ts.net:8443".into(),
            ],
            build: "0.6.0-4f2a9c1".into(),
            schema: SCHEMA,
        },
        signin: signin::request(&browser, "https://coeus.example.com", NOW),
        signed_in: SignedIn {
            token: "v1.eyJ2IjoxfQ.c2lnbmF0dXJl".into(),
            viewer: Viewer {
                login: "r3dlust@github".into(),
                device: "iphone".into(),
                expires_at: Timestamp(NOW.0 + 7 * DAY),
            },
        },
    };

    #[derive(serde::Serialize)]
    struct Contract {
        fleet: Vec<FleetEvent>,
        log: Vec<LogEvent>,
        diff: DiffView,
        auth: Auth,
    }
    let contract = Contract {
        fleet,
        log,
        diff: DiffView::from(&diff()),
        auth,
    };
    serde_json::to_string_pretty(&contract).unwrap() + "\n"
}

fn collect(value: &Value, strings: &mut BTreeSet<String>) {
    match value {
        Value::String(string) => {
            strings.insert(string.clone());
        }
        Value::Array(values) => values.iter().for_each(|value| collect(value, strings)),
        Value::Object(map) => {
            for (key, value) in map {
                strings.insert(key.clone());
                collect(value, strings);
            }
        }
        _ => {}
    }
}
