//! when a host counts as ready to serve

use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    time::Duration,
};

use tokio::{sync::watch, time::Instant};
use tokio_util::sync::CancellationToken;

use crate::{
    common::proto::Role,
    config::{ClientSettings, Endpoint, Key, ReadinessSettings},
    server::CoeusServer,
    testing::Harness,
    web::ready::{self, Edge, Readiness},
};

/// the readiness after `probes`, and the edges they crossed, by probe number
fn run(probes: &[bool]) -> (Readiness, Vec<(usize, Edge)>) {
    let settings = ReadinessSettings::default();
    let mut state = Readiness::START;
    let mut edges = Vec::new();
    for (number, passed) in probes.iter().enumerate() {
        let (next, edge) = state.step(*passed, &settings);
        state = next;
        edges.extend(edge.map(|edge| (number + 1, edge)));
    }
    (state, edges)
}

#[test]
fn a_host_is_ready_after_three_passes_in_a_row() {
    assert_eq!(run(&[true, true]), (Readiness::Down { passes: 2 }, vec![]));
    assert_eq!(
        run(&[true, true, true]),
        (Readiness::Up { fails: 0 }, vec![(3, Edge::Rose)])
    );
    assert_eq!(
        run(&[true, true, false, true, true, true]),
        (Readiness::Up { fails: 0 }, vec![(6, Edge::Rose)]),
        "a fail starts the count over"
    );
}

#[test]
fn a_ready_host_stops_after_five_fails_in_a_row() {
    let up = [true; 3];
    let flaky = [up.as_slice(), &[false; 4], &[true], &[false; 4]].concat();
    assert_eq!(
        run(&flaky),
        (Readiness::Up { fails: 4 }, vec![(3, Edge::Rose)]),
        "a pass starts the count over"
    );

    let gone = [up.as_slice(), &[false; 5]].concat();
    assert_eq!(
        run(&gone),
        (Readiness::START, vec![(3, Edge::Rose), (8, Edge::Fell)])
    );
}

/// a probe that answers with `verdicts` in turn, then passes; `None` hangs
fn scripted(
    verdicts: &[Option<bool>],
) -> impl FnMut() -> futures_util::future::BoxFuture<'static, Result<(), String>> {
    let verdicts = Arc::new(Mutex::new(
        verdicts.iter().copied().collect::<VecDeque<_>>(),
    ));
    move || {
        let verdict = verdicts.lock().unwrap().pop_front().unwrap_or(Some(true));
        Box::pin(async move {
            match verdict {
                Some(true) => Ok(()),
                Some(false) => Err("tailscale is Stopped".to_owned()),
                None => std::future::pending().await,
            }
        })
    }
}

/// when `ready` flips from what it holds, after `start`
async fn flip(ready: &mut watch::Receiver<bool>, start: Instant) -> (bool, Duration) {
    ready.changed().await.unwrap();
    (*ready.borrow_and_update(), start.elapsed())
}

#[tokio::test(start_paused = true)]
async fn readiness_is_published_on_its_edges_at_the_probe_interval() {
    let start = Instant::now();
    let (sender, mut ready) = watch::channel(false);
    let probe = scripted(&[
        Some(false),
        Some(true),
        Some(true),
        Some(true),
        None,
        Some(false),
        None,
        Some(false),
        Some(false),
    ]);
    tokio::spawn(ready::watch(ReadinessSettings::default(), probe, sender));

    let second = Duration::from_secs(1);
    assert_eq!(
        flip(&mut ready, start).await,
        (true, 6 * second),
        "probes at 0, 2, 4 and 6 s, the first failing"
    );
    assert_eq!(
        flip(&mut ready, start).await,
        (false, 16 * second),
        "a probe that hangs fails once the interval is up"
    );
    assert_eq!(flip(&mut ready, start).await, (true, 22 * second));
}

#[tokio::test(start_paused = true)]
async fn readiness_stops_probing_once_nobody_listens() {
    let (sender, ready) = watch::channel(false);
    let watching = tokio::spawn(ready::watch(
        ReadinessSettings::default(),
        scripted(&[]),
        sender,
    ));
    drop(ready);
    tokio::time::timeout(Duration::from_secs(5), watching)
        .await
        .unwrap()
        .unwrap();
}

/// coeusd on `port` with the read key, until `stop`
async fn serve(harness: &Harness, read_key: &Key, port: u16, stop: CancellationToken) -> u16 {
    let keys = [
        (Role::Full, Key::generate()),
        (Role::Read, read_key.clone()),
    ];
    let address = format!("127.0.0.1:{port}").parse().unwrap();
    let server = CoeusServer::bind(address, &keys, harness.engine.clone())
        .await
        .unwrap();
    let port = server.local_addr().unwrap().port();
    tokio::spawn(server.run(stop));
    port
}

#[tokio::test]
async fn a_coeusd_restart_stays_inside_the_window_a_ready_host_keeps_serving_through() {
    let harness = Harness::start(tempfile::tempdir().unwrap()).await;
    let read_key = Key::generate();
    let stop = CancellationToken::new();
    let port = serve(&harness, &read_key, 0, stop.clone()).await;

    let (sender, mut ready) = watch::channel(false);
    let daemon = Endpoint {
        host: "127.0.0.1".into(),
        port,
    };
    let client = ClientSettings::default();
    let key = read_key.clone();
    tokio::spawn(async move {
        ready::watch(
            ReadinessSettings::default(),
            || ready::answers(&client, &daemon, &key),
            sender,
        )
        .await;
    });
    ready.changed().await.unwrap();
    assert!(*ready.borrow_and_update());

    // coeusd drains and exits, and systemd starts it again 2 s later
    stop.cancel();
    tokio::time::sleep(Duration::from_secs(2)).await;
    let stop = CancellationToken::new();
    serve(&harness, &read_key, port, stop.clone()).await;
    let daemon = Endpoint {
        host: "127.0.0.1".into(),
        port,
    };
    ready::answers(&ClientSettings::default(), &daemon, &read_key)
        .await
        .unwrap();

    // one more probe, to be sure the restarted coeusd counted
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert!(!ready.has_changed().unwrap(), "readiness never fell");
    stop.cancel();
    harness.shutdown.cancel();
}

#[tokio::test]
async fn a_coeusd_that_does_not_answer_fails_the_probe() {
    let daemon = Endpoint {
        host: "127.0.0.1".into(),
        port: tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .unwrap()
            .local_addr()
            .unwrap()
            .port(),
    };
    let reason = ready::answers(&ClientSettings::default(), &daemon, &Key::generate())
        .await
        .unwrap_err();
    assert!(
        reason.starts_with("the local coeusd is unreachable"),
        "{reason}"
    );
}
