use crate::{
    bootstrap::switch::{
        AdoptLine, Feed,
        ship::{Archived, Lock},
        stock::{Waiting, built_path, drain, remote_program},
    },
    common::model::{
        Action, JobEvent, JobId, JobState, JobSummary, OrderId, Origin, Outcome, Phase, Progress,
        Source, Stamped, Tally, Timestamp,
    },
};

fn job() -> JobSummary {
    JobSummary {
        id: JobId(1),
        order: OrderId(0x5f0e),
        source: Source::Snapshot {
            nar_hash: "sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="
                .parse()
                .unwrap(),
            label: "altar:coeus/bootstrap/nova@a1b2c3d".into(),
        },
        action: Action::Switch,
        origin: Origin::User {
            user: "r3dlust".into(),
            host: "altar".into(),
        },
        state: JobState::Queued,
        ordered_at: Timestamp(1_000),
        started_at: None,
        finished_at: None,
        toplevel: None,
        attempt: 0,
    }
}

fn event(at: i64, event: JobEvent) -> String {
    format!(
        "@coeus {}",
        serde_json::to_string(&Stamped {
            at: Timestamp(at),
            event
        })
        .unwrap()
    )
}

#[test]
fn adopts_lines_are_read_by_their_prefix() {
    let job_line = format!("@job {}", serde_json::to_string(&job()).unwrap());
    assert_eq!(AdoptLine::parse(&job_line), AdoptLine::Job(Box::new(job())));

    let phase = event(2_000, JobEvent::Phase(Phase::Building));
    assert_eq!(
        AdoptLine::parse(&phase),
        AdoptLine::Event(Stamped {
            at: Timestamp(2_000),
            event: JobEvent::Phase(Phase::Building)
        })
    );

    let nix = r#"@nix {"action":"msg","level":3,"msg":"hello"}"#;
    assert_eq!(AdoptLine::parse(nix), AdoptLine::Nix(nix.into()));
    assert_eq!(AdoptLine::parse("@at 214\n"), AdoptLine::At(214));

    for other in [
        "@at nowhere",
        "@job {",
        "@coeus {}",
        "@future {}",
        "plain output",
        "",
    ] {
        assert_eq!(AdoptLine::parse(other), AdoptLine::Other, "{other}");
    }
}

#[test]
fn a_feed_folds_the_job_until_it_finishes() {
    let mut feed = Feed::default();
    let lines = [
        format!("@job {}", serde_json::to_string(&job()).unwrap()),
        event(
            2_000,
            JobEvent::Started {
                attempt: 1,
                pid: 4121,
                version: "0.2.0".into(),
            },
        ),
        "@at 120".into(),
        event(3_000, JobEvent::Phase(Phase::Building)),
        event(
            3_500,
            JobEvent::Progress(Progress {
                builds: Tally {
                    done: 3,
                    expected: 10,
                    ..Tally::default()
                },
                ..Progress::default()
            }),
        ),
        "@at 300".into(),
    ];
    for line in &lines {
        feed.line(AdoptLine::parse(line));
    }

    let view = feed.view.as_ref().unwrap();
    assert_eq!(
        view.summary.state,
        JobState::Running {
            phase: Phase::Building
        }
    );
    assert_eq!(view.progress.builds.done, 3);
    assert_eq!(feed.offset, 300);
    assert_eq!(feed.finished(), None);

    feed.line(AdoptLine::parse(&event(
        9_000,
        JobEvent::Finished(Outcome::Succeeded),
    )));
    let finished = feed.finished().unwrap();
    assert_eq!(finished.state, JobState::Finished(Outcome::Succeeded));
    assert_eq!(finished.finished_at, Some(Timestamp(9_000)));
}

#[test]
fn a_resumed_follow_keeps_the_progress_it_had() {
    let mut feed = Feed::default();
    feed.line(AdoptLine::Job(Box::new(job())));
    feed.line(AdoptLine::parse(&event(
        3_500,
        JobEvent::Progress(Progress {
            downloaded_bytes: 42,
            ..Progress::default()
        }),
    )));

    let running = JobSummary {
        state: JobState::Running {
            phase: Phase::Activating,
        },
        ..job()
    };
    feed.line(AdoptLine::Job(Box::new(running.clone())));
    let view = feed.view.as_ref().unwrap();
    assert_eq!(view.summary, running);
    assert_eq!(view.progress.downloaded_bytes, 42);
}

#[tokio::test]
async fn adopts_output_is_read_to_its_end_after_the_job_is_seen_finished() {
    let finished = JobSummary {
        state: JobState::Finished(Outcome::Failed(crate::common::model::Failure::new(
            Phase::Building,
            "builder failed",
        ))),
        ..job()
    };
    let mut output = format!("@job {}\n", serde_json::to_string(&finished).unwrap());
    // the log from the offset asked, replayed after the summary
    output.push_str(&event(2_000, JobEvent::Phase(Phase::Building)));
    output.push('\n');
    for n in 0..2_000 {
        output.push_str(&format!(
            "@nix {{\"action\":\"msg\",\"level\":0,\"msg\":\"error: line {n}\"}}\n@at {n}\n"
        ));
    }

    // a pipe far smaller than the output: adopt finishes writing only if
    // everything is read
    let (mut adopt, stdout) = tokio::io::duplex(256);
    let writing = tokio::spawn(async move {
        tokio::io::AsyncWriteExt::write_all(&mut adopt, output.as_bytes()).await
    });

    let mut feed = Feed::default();
    tokio::time::timeout(
        std::time::Duration::from_secs(10),
        drain(stdout, &mut feed, |_| {}),
    )
    .await
    .expect("the output was read to its end");
    writing.await.unwrap().unwrap();

    assert_eq!(
        feed.finished(),
        Some(&finished),
        "replayed events are older"
    );
    assert_eq!(feed.offset, 1_999);
    assert!(feed.failure().iter().any(|line| line.contains("line 1999")));
}

#[test]
fn nix_lines_explain_a_failure() {
    let mut feed = Feed::default();
    feed.line(AdoptLine::parse(
        r#"@nix {"action":"msg","level":0,"msg":"error: builder for thoth-0.4 failed with exit code 1"}"#,
    ));
    let failure = feed.failure();
    assert!(
        failure.iter().any(|line| line.contains("thoth-0.4")),
        "{failure:?}"
    );
}

#[test]
fn what_nix_prints_names_the_store_paths() {
    let archived = r#"{"inputs":{"nixpkgs":{"inputs":{},"path":"/nix/store/0123456789abcdfghijklmnpqrsvwxyz-source"}},"path":"/nix/store/abcdfghijklmnpqrsvwxyz0123456789-source"}"#;
    assert_eq!(
        Archived::parse(archived).unwrap().path.as_str(),
        "/nix/store/abcdfghijklmnpqrsvwxyz0123456789-source"
    );

    let built = r#"[{"drvPath":"/nix/store/0123456789abcdfghijklmnpqrsvwxyz-coeus-0.2.0.drv","outputs":{"out":"/nix/store/0123456789abcdfghijklmnpqrsvwxyz-coeus-0.2.0"}}]"#;
    assert_eq!(
        built_path(built).unwrap().as_str(),
        "/nix/store/0123456789abcdfghijklmnpqrsvwxyz-coeus-0.2.0"
    );
    assert!(built_path("[]").is_err());
}

#[test]
fn only_the_clone_and_inputs_the_machine_cannot_fetch_are_shipped() {
    let store = |name: &str| format!("/nix/store/{name:a>32}-source");
    let archived = format!(
        r#"{{"path":"{root}","inputs":{{
            "nixpkgs":{{"path":"{nixpkgs}","inputs":{{}}}},
            "bot":{{"path":"{bot}","inputs":{{"nixpkgs":{{"path":"{botpkgs}","inputs":{{}}}}}}}},
            "local":{{"path":"{local}","inputs":{{}}}},
            "mirror":{{"path":"{mirror}","inputs":{{}}}}
        }}}}"#,
        root = store("root"),
        nixpkgs = store("nixpkgs"),
        bot = store("bot"),
        botpkgs = store("botpkgs"),
        local = store("local"),
        mirror = store("mirror"),
    );
    let lock = r#"{"root":"root","version":7,"nodes":{
        "root":{"inputs":{"nixpkgs":"nixpkgs","bot":"bot","local":"local","mirror":"mirror"}},
        "nixpkgs":{"locked":{"type":"github","owner":"NixOS","repo":"nixpkgs"}},
        "bot":{"inputs":{"nixpkgs":"nixpkgs_2","shared":["nixpkgs"]},
               "locked":{"type":"git","url":"ssh://git@github.com/owner/bot"}},
        "nixpkgs_2":{"locked":{"type":"github","owner":"NixOS","repo":"nixpkgs"}},
        "local":{"locked":{"type":"path","path":"/home/me/local"}},
        "mirror":{"locked":{"type":"git","url":"https://example.org/mirror.git"}}
    }}"#;
    let shipped = Lock::parse(lock)
        .unwrap()
        .private_paths(&Archived::parse(&archived).unwrap());
    let shipped: Vec<&str> = shipped.iter().map(|path| path.as_str()).collect();
    assert_eq!(shipped[0], store("root"), "the clone goes first");
    assert!(shipped.contains(&store("bot").as_str()), "{shipped:?}");
    assert!(shipped.contains(&store("local").as_str()), "{shipped:?}");
    assert_eq!(shipped.len(), 3, "public inputs stay behind: {shipped:?}");
}

#[test]
fn waits_double_up_to_their_ceiling_and_start_over_after_a_success() {
    use std::time::Duration;

    use crate::bootstrap::switch::backoff::Backoff;

    let mut backoff = Backoff::new(Duration::from_secs(5), Duration::from_secs(30));
    let waits: Vec<u64> = (0..5).map(|_| backoff.next().as_secs()).collect();
    assert_eq!(waits, [5, 10, 20, 30, 30]);
    backoff.reset();
    assert_eq!(backoff.next(), Duration::from_secs(5));
}

#[test]
fn waiting_for_full_disk_access_outlasts_a_dropped_connection_only() {
    use std::{os::unix::process::ExitStatusExt, process::ExitStatus};

    use crate::adopt::NEEDS_FULL_DISK_ACCESS;

    let exited = |code: i32| ExitStatus::from_raw(code << 8);
    let fresh = Feed::default();

    let refused = exited(NEEDS_FULL_DISK_ACCESS);
    assert_eq!(Waiting::after(refused, false, &fresh), Waiting::ForGrant);
    assert_eq!(Waiting::after(exited(255), true, &fresh), Waiting::ForSsh);
    assert_eq!(Waiting::after(exited(255), false, &fresh), Waiting::Over);
    assert_eq!(Waiting::after(exited(0), true, &fresh), Waiting::Over);

    // once the job streams, a drop is the switch's, for the rejoin
    let mut streaming = Feed::default();
    streaming.line(AdoptLine::Job(Box::new(job())));
    assert_eq!(Waiting::after(exited(255), true, &streaming), Waiting::Over);
}

#[test]
fn the_shipped_inputs_reach_a_nix_daemon_run_as_root() {
    use zeroize::Zeroizing;

    use crate::bootstrap::remote::{Platform, Privilege};

    let nixos = "/run/current-system/sw/bin/nix-daemon";
    assert_eq!(remote_program(Platform::Nixos, &Privilege::Root), nixos);
    assert_eq!(
        remote_program(Platform::Nixos, &Privilege::Sudo),
        format!("sudo -n {nixos}")
    );
    assert_eq!(
        remote_program(Platform::Darwin, &Privilege::Sudo),
        "sudo -n /nix/var/nix/profiles/default/bin/nix-daemon"
    );

    // sudo cannot be fed a password through nix: the login ships as itself
    let password = Privilege::SudoPassword(Zeroizing::new("hunter2".into()));
    assert_eq!(remote_program(Platform::Nixos, &password), nixos);
}

mod first {
    use super::job;
    use crate::{
        bootstrap::{
            remote::Running,
            switch::first::{Back, Next, Refusal, goes_on, owes_reboot},
        },
        common::model::{
            Action, Attempt, Failure, JobState, JobSummary, OrderId, Outcome, Phase, Platform,
            StorePath,
        },
    };

    const NEXT: OrderId = OrderId(0x5f0f);

    fn ended(action: Action, outcome: Outcome) -> JobSummary {
        JobSummary {
            action,
            state: JobState::Finished(outcome),
            toplevel: Some(fleet()),
            ..job()
        }
    }

    fn fleet() -> StorePath {
        "/nix/store/0123456789abcdfghijklmnpqrsvwxyz-nixos-system-nova-26.05"
            .parse()
            .unwrap()
    }

    fn stock() -> StorePath {
        "/nix/store/abcdfghijklmnpqrsvwxyz0123456789-nixos-system-nixos-25.11"
            .parse()
            .unwrap()
    }

    /// what switch-to-configuration prints when a pre-switch check refuses
    fn refused(phase: Phase, rolled_back: bool) -> JobSummary {
        let failure = Failure {
            phase,
            message: "switch-to-configuration failed with exit status: 1".into(),
            tail: vec![
                "copying the system".into(),
                "Checking switch inhibitors...".into(),
                "jq: error (at /run/current-system/switch-inhibitors:0): Cannot iterate over null"
                    .into(),
                "".into(),
                "Pre-switch check 'switchInhibitors' failed".into(),
                "Pre-switch checks failed".into(),
            ],
        };
        let outcome = match rolled_back {
            true => Outcome::RolledBack(failure),
            false => Outcome::Failed(failure),
        };
        ended(Action::Switch, outcome)
    }

    fn failed(last: JobSummary) -> Attempt {
        Attempt::Failed {
            last,
            next: Some(NEXT),
        }
    }

    #[test]
    fn a_switch_nixos_refuses_is_booted_by_the_next_try_without_asking() {
        for rolled_back in [true, false] {
            let next = Next::of(
                failed(refused(Phase::Activating, rolled_back)),
                Platform::NixOs,
                false,
            );
            let Next::Boot { order, refusal } = next else {
                panic!("expected a boot, got {next:?}");
            };
            assert_eq!(order, NEXT);
            assert_eq!(
                refusal.lines,
                [
                    "Checking switch inhibitors...",
                    "jq: error (at /run/current-system/switch-inhibitors:0): Cannot iterate over null",
                    "Pre-switch check 'switchInhibitors' failed",
                ]
            );
            assert!(refusal.note().starts_with(
                "NixOS asks for a reboot into this system instead of a live switch: Checking switch inhibitors..."
            ));
        }

        let last = Attempt::Failed {
            last: refused(Phase::Activating, true),
            next: None,
        };
        assert_eq!(Next::of(last, Platform::NixOs, false), Next::Exhausted);
    }

    #[test]
    fn other_failures_wait_for_retry() {
        let guard = ended(
            Action::Switch,
            Outcome::RolledBack(Failure::new(Phase::Verifying, "coeusd does not answer")),
        );
        assert_eq!(
            Next::of(failed(guard.clone()), Platform::NixOs, false),
            Next::Failed(guard.clone())
        );
        assert_eq!(
            Next::of(failed(guard), Platform::NixOs, true),
            Next::Order {
                order: NEXT,
                action: Action::Switch
            }
        );

        let building = refused(Phase::Building, false);
        assert!(matches!(
            Next::of(failed(building), Platform::NixOs, false),
            Next::Failed(_)
        ));

        // a Mac has no pre-switch checks and no boot action
        let mac = refused(Phase::Activating, true);
        assert_eq!(Refusal::of(&mac, Platform::Darwin), None);
        assert!(matches!(
            Next::of(failed(mac), Platform::Darwin, false),
            Next::Failed(_)
        ));

        // a boot that failed is booted again on retry, never switched
        let boot = ended(
            Action::Boot,
            Outcome::Failed(Failure::new(Phase::Activating, "no space left on /boot")),
        );
        assert_eq!(
            Next::of(failed(boot), Platform::NixOs, true),
            Next::Order {
                order: NEXT,
                action: Action::Boot
            }
        );
    }

    #[test]
    fn a_boot_that_succeeded_is_done_once_the_machine_runs_its_system() {
        let boot = ended(Action::Boot, Outcome::Succeeded);
        assert_eq!(
            Next::of(Attempt::Succeeded(boot.clone()), Platform::NixOs, false),
            Next::Booted(boot.clone())
        );
        assert!(owes_reboot(&boot, Some(&stock())));
        assert!(owes_reboot(&boot, None));
        assert!(!owes_reboot(&boot, Some(&fleet())));

        let switch = ended(Action::Switch, Outcome::Succeeded);
        assert_eq!(
            Next::of(Attempt::Succeeded(switch.clone()), Platform::NixOs, false),
            Next::Done(switch)
        );
        let running = JobSummary {
            action: Action::Boot,
            ..job()
        };
        assert_eq!(
            Next::of(Attempt::Running(running.clone()), Platform::NixOs, false),
            Next::Follow(running)
        );
    }

    #[test]
    fn only_a_refusal_or_a_boot_leaves_more_to_do_after_a_try() {
        assert!(goes_on(&refused(Phase::Activating, true), Platform::NixOs));
        assert!(goes_on(
            &ended(Action::Boot, Outcome::Succeeded),
            Platform::NixOs
        ));
        assert!(!goes_on(
            &ended(Action::Switch, Outcome::Succeeded),
            Platform::NixOs
        ));
        assert!(!goes_on(
            &ended(
                Action::Boot,
                Outcome::Failed(Failure::new(Phase::Activating, "boom"))
            ),
            Platform::NixOs
        ));
        assert!(!goes_on(
            &refused(Phase::Activating, true),
            Platform::Darwin
        ));
    }

    #[test]
    fn a_rebooted_machine_is_home_only_on_the_boot_jobs_system() {
        let boot = ended(Action::Boot, Outcome::Succeeded);
        let before = Running {
            system: Some(stock()),
            boot: Some("first".into()),
        };
        let now = |system: StorePath, boot: &str| Running {
            system: Some(system),
            boot: Some(boot.into()),
        };

        assert_eq!(Back::of(&boot, &before, &before), Back::NotYet);
        assert_eq!(
            Back::of(&boot, &before, &now(fleet(), "second")),
            Back::Home
        );
        assert_eq!(
            Back::of(&boot, &before, &now(stock(), "second")),
            Back::Elsewhere(Some(stock()))
        );
    }
}
