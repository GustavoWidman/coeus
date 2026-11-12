use super::*;

const SHA: &str = "caf4e2b9a1d6c3e8f0b2a4c6d8e0f1a3b5c7d9e1";
const TOPLEVEL: &str = "/nix/store/h0myp2wfggr0x8pfacciv8sfhxyb845d-nixos-system-lab-26.11";

fn summary() -> JobSummary {
    JobSummary {
        id: JobId(42),
        order: OrderId(7),
        source: Source::Git {
            rev: SHA.parse().unwrap(),
            git_ref: Some("main".into()),
        },
        action: Action::Switch,
        origin: Origin::Follow,
        state: JobState::Queued,
        ordered_at: Timestamp(1_000),
        started_at: None,
        finished_at: None,
        toplevel: None,
        attempt: 0,
    }
}

#[test]
fn parses_job_ids_with_or_without_a_hash() {
    assert_eq!("#42".parse::<JobId>().unwrap(), JobId(42));
    assert_eq!("42".parse::<JobId>().unwrap(), JobId(42));
    assert_eq!(JobId(42).to_string(), "#42");
    assert!("#x".parse::<JobId>().is_err());
}

#[test]
fn commit_hashes_round_trip_and_shorten() {
    let sha: CommitSha = SHA.parse().unwrap();
    assert_eq!(sha.to_string(), SHA);
    assert_eq!(sha.short(), "caf4e2b");
    assert!("caf4e2b".parse::<CommitSha>().is_err());
    assert!(SHA.replace('c', "z").parse::<CommitSha>().is_err());
}

#[test]
fn store_paths_expose_their_name() {
    let path: StorePath = TOPLEVEL.parse().unwrap();
    assert_eq!(path.name(), "nixos-system-lab-26.11");
    assert!("/tmp/nix/store/x".parse::<StorePath>().is_err());
    assert!("/nix/store/short-name".parse::<StorePath>().is_err());
}

#[test]
fn nar_hashes_become_single_path_components() {
    let hash = NarHash::try_from("sha256-Qk3+/abcdefghijklmnopqrstuvwxyz0123456789AB=".to_owned())
        .unwrap();
    assert!(!hash.file_name().contains('/'));
    assert!(NarHash::try_from("sha256-../../etc".to_owned()).is_err());
}

#[test]
fn sources_read_like_refs() {
    assert_eq!(summary().source.to_string(), "main@caf4e2b");
    assert_eq!(Source::Generation(41).to_string(), "generation 41");
}

#[test]
fn only_phases_before_activation_are_cancellable() {
    assert!(Phase::Building.is_cancellable());
    assert!(!Phase::Activating.is_cancellable());
    assert!(!Phase::Verifying.is_cancellable());
}

#[test]
fn a_job_view_folds_its_events() {
    let mut view = JobView::queued(summary());
    let events = [
        Stamped {
            at: Timestamp(2_000),
            event: JobEvent::Started {
                attempt: 1,
                pid: 99,
                version: "0.2.0".into(),
            },
        },
        Stamped {
            at: Timestamp(3_000),
            event: JobEvent::Phase(Phase::Building),
        },
        Stamped {
            at: Timestamp(3_500),
            event: JobEvent::Evaluated {
                drv: TOPLEVEL.parse().unwrap(),
                toplevel: TOPLEVEL.parse().unwrap(),
            },
        },
        Stamped {
            at: Timestamp(9_000),
            event: JobEvent::Finished(Outcome::Succeeded),
        },
    ];

    for event in &events[..2] {
        view.apply(event);
    }
    assert_eq!(view.summary.state.phase(), Some(Phase::Building));
    assert_eq!(view.phase_started_at, Some(Timestamp(3_000)));

    for event in &events[2..] {
        view.apply(event);
    }
    assert_eq!(view.summary.state, JobState::Finished(Outcome::Succeeded));
    assert_eq!(view.summary.toplevel.as_ref().unwrap().as_str(), TOPLEVEL);
    assert_eq!(view.summary.duration(), Some(std::time::Duration::from_secs(7)));
}

#[test]
fn job_events_keep_a_stable_json_shape() {
    let stamped = Stamped {
        at: Timestamp(1_794_443_502_123),
        event: JobEvent::Phase(Phase::Activating),
    };

    let json = serde_json::to_string(&stamped).unwrap();
    assert_eq!(json, r#"{"at":1794443502123,"event":{"phase":"Activating"}}"#);
    assert_eq!(serde_json::from_str::<Stamped<JobEvent>>(&json).unwrap(), stamped);
}

#[test]
fn progress_is_unknown_until_nix_announces_work() {
    let mut progress = Progress::default();
    assert_eq!(progress.fraction(), None);

    progress.builds = Tally {
        done: 1,
        expected: 4,
        running: 1,
        failed: 0,
    };
    progress.downloads = Tally {
        done: 3,
        expected: 4,
        running: 0,
        failed: 0,
    };
    assert_eq!(progress.fraction(), Some(0.5));
}
