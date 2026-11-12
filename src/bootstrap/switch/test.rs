use crate::{
    bootstrap::switch::{
        AdoptLine, Feed,
        stock::{archived_path, built_path},
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
        archived_path(archived).unwrap().as_str(),
        "/nix/store/abcdfghijklmnpqrsvwxyz0123456789-source"
    );

    let built = r#"[{"drvPath":"/nix/store/0123456789abcdfghijklmnpqrsvwxyz-coeus-0.2.0.drv","outputs":{"out":"/nix/store/0123456789abcdfghijklmnpqrsvwxyz-coeus-0.2.0"}}]"#;
    assert_eq!(
        built_path(built).unwrap().as_str(),
        "/nix/store/0123456789abcdfghijklmnpqrsvwxyz-coeus-0.2.0"
    );
    assert!(built_path("[]").is_err());
}
