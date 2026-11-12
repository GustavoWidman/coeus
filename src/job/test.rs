use std::io::Write;

use super::*;
use crate::common::model::{
    Action, JobEvent, JobId, JobState, NixOptions, Order, OrderId, Origin, Outcome, Phase,
    Platform, Source, Timestamp,
};

fn spec(id: u64) -> JobSpec {
    JobSpec {
        id: JobId(id),
        order: Order {
            id: OrderId(id),
            source: Source::Generation(3),
            action: Action::Switch,
            options: NixOptions::default(),
            origin: Origin::Follow,
            now: false,
        },
        ordered_at: Timestamp(1_000),
        target: Target {
            hostname: "lab".parse().unwrap(),
            platform: Platform::NixOs,
            flake: "git+https://example.com/nix".into(),
            sources: "/var/lib/coeus/sources".into(),
        },
        ssh_key_file: None,
        guard: None,
    }
}

fn store() -> (tempfile::TempDir, JobStore) {
    let state = tempfile::tempdir().unwrap();
    let store = JobStore::open(state.path()).unwrap();
    (state, store)
}

#[test]
fn numbers_jobs_in_order_and_prunes_the_oldest_finished() {
    let (_state, store) = store();
    assert_eq!(store.next_id().unwrap(), JobId(1));

    for id in 1..=4 {
        store.create(&spec(id)).unwrap();
    }
    assert_eq!(store.next_id().unwrap(), JobId(5));

    store.prune(1, |id| id != JobId(4)).unwrap();
    assert_eq!(store.list().unwrap(), [JobId(3), JobId(4)]);
}

#[test]
fn a_log_has_one_writer_at_a_time() {
    let (_state, store) = store();
    let log = store.create(&spec(1)).unwrap().log();

    let writer = log
        .writer()
        .unwrap()
        .expect("the first writer gets the log");
    assert!(log.is_held().unwrap());
    assert!(
        log.writer().unwrap().is_none(),
        "a second writer must be refused"
    );

    drop(writer);
    assert!(!log.is_held().unwrap());
    assert!(log.writer().unwrap().is_some());
}

#[tokio::test]
async fn wraps_plain_output_and_keeps_nix_lines() {
    let (_state, store) = store();
    let log = store.create(&spec(1)).unwrap().log();
    let mut writer = log.writer().unwrap().unwrap();

    writer.output(r#"@nix {"action":"stop","id":1}"#).unwrap();
    writer
        .output("stopping the following units: \"coeusd\"\n")
        .unwrap();
    writer.output(&"x".repeat(JobLog::MAX_LINE * 2)).unwrap();
    writer.flush().unwrap();

    let mut reader = log.reader(0).await.unwrap();
    let Some(LogItem::Nix(lines)) = reader.next(usize::MAX).await.unwrap() else {
        panic!("expected nix lines");
    };
    let lines: Vec<&str> = std::str::from_utf8(&lines).unwrap().lines().collect();

    assert_eq!(lines[0], r#"@nix {"action":"stop","id":1}"#);
    assert_eq!(
        lines[1],
        r#"@nix {"action":"msg","level":3,"msg":"stopping the following units: \"coeusd\""}"#
    );
    assert!(lines[2].len() <= JobLog::MAX_LINE);
}

#[tokio::test]
async fn readers_resume_and_never_take_a_partial_line() {
    let (_state, store) = store();
    let dir = store.create(&spec(1)).unwrap();
    let log = dir.log();
    let mut writer = log.writer().unwrap().unwrap();

    writer.event(JobEvent::Phase(Phase::Building)).unwrap();
    writer.output(r#"@nix {"action":"stop","id":1}"#).unwrap();
    writer.flush().unwrap();

    let mut partial = std::fs::OpenOptions::new()
        .append(true)
        .open(dir.path().join("log"))
        .unwrap();
    partial.write_all(br#"@nix {"action":"st"#).unwrap();

    let mut reader = log.reader(0).await.unwrap();
    assert!(matches!(
        reader.next(usize::MAX).await.unwrap(),
        Some(LogItem::Event(event)) if event.event == JobEvent::Phase(Phase::Building)
    ));
    assert!(matches!(
        reader.next(usize::MAX).await.unwrap(),
        Some(LogItem::Nix(_))
    ));
    assert_eq!(reader.next(usize::MAX).await.unwrap(), None);

    let resume = reader.offset();
    partial.write_all(b"op\",\"id\":2}\n").unwrap();

    let mut resumed = log.reader(resume).await.unwrap();
    assert_eq!(
        resumed.next(usize::MAX).await.unwrap(),
        Some(LogItem::Nix(
            br#"@nix {"action":"stop","id":2}"#.iter().copied().chain(*b"\n").collect()
        ))
    );
}

#[tokio::test]
async fn batches_nix_lines_up_to_the_limit() {
    let (_state, store) = store();
    let log = store.create(&spec(1)).unwrap().log();
    let mut writer = log.writer().unwrap().unwrap();

    let line = r#"@nix {"action":"stop","id":1}"#;
    for _ in 0..10 {
        writer.output(line).unwrap();
    }
    writer.event(JobEvent::Note("done".into())).unwrap();

    let mut reader = log.reader(0).await.unwrap();
    let limit = (line.len() + 1) * 4;
    let mut batches = Vec::new();
    while let Some(LogItem::Nix(batch)) = reader.next(limit).await.unwrap() {
        assert!(batch.len() <= limit);
        batches.push(batch.len() / (line.len() + 1));
    }

    assert_eq!(batches, [4, 4, 2]);
}

#[tokio::test]
async fn a_view_folds_the_log_until_a_summary_exists() {
    let (_state, store) = store();
    let dir = store.create(&spec(1)).unwrap();
    assert_eq!(dir.view().await.unwrap().summary.state, JobState::Queued);

    let mut writer = dir.log().writer().unwrap().unwrap();
    writer
        .event(JobEvent::Started {
            attempt: 1,
            pid: 1,
            version: "0.0.0".into(),
        })
        .unwrap();
    writer.event(JobEvent::Phase(Phase::Activating)).unwrap();
    assert_eq!(
        dir.view().await.unwrap().summary.state.phase(),
        Some(Phase::Activating)
    );

    writer.finish(Outcome::Succeeded).unwrap();
    let mut finished = dir.view().await.unwrap().summary;
    assert_eq!(finished.state, JobState::Finished(Outcome::Succeeded));

    finished.attempt = 9;
    dir.write_summary(&finished).unwrap();
    assert_eq!(dir.view().await.unwrap().summary.attempt, 9);
}

#[test]
fn stop_requests_round_trip() {
    let (_state, store) = store();
    let dir = store.create(&spec(1)).unwrap();
    assert_eq!(dir.stop_request().unwrap(), None);

    dir.request_stop(StopRequest::Superseded { by: JobId(2) })
        .unwrap();
    assert_eq!(
        dir.stop_request().unwrap().map(StopRequest::outcome),
        Some(Outcome::Superseded { by: JobId(2) })
    );
}

#[test]
fn concurrent_summary_writers_never_trip_over_each_other() {
    let (_state, store) = store();
    let dir = store.create(&spec(1)).unwrap();
    let summary = dir.queued().unwrap().summary;

    let writers: Vec<_> = (0..2)
        .map(|_| {
            let (dir, summary) = (dir.clone(), summary.clone());
            std::thread::spawn(move || {
                for _ in 0..200 {
                    dir.write_summary(&summary).unwrap();
                }
            })
        })
        .collect();

    for writer in writers {
        writer.join().expect("a writer failed");
    }
    assert_eq!(dir.summary().unwrap(), Some(summary));
}
