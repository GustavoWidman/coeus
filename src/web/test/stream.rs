//! the fleet stream's promises, timed on tokio's paused clock

use std::time::Duration;

use tokio::time::{Instant, sleep};

use super::{
    NOW, release, running,
    sse::{Events, Message, link, site, view, viewer},
    view as machine,
};
use crate::{
    client::Link,
    common::model::{JobView, MachineView, Tally},
    web::{site::Access, stream::PROGRESS},
};

const QUIET: Duration = Duration::from_secs(1);

/// lab building job 61 of a deployment, `done` of its 388 derivations built
fn building(done: u32) -> MachineView {
    let mut lab = machine("lab");
    let mut job = JobView::queued(release(61, 0xd1, 0, running()));
    job.progress.builds = Tally {
        done,
        expected: 388,
        running: 2,
        failed: 0,
    };
    lab.running = Some(job);
    lab
}

fn open() -> crate::web::site::Site {
    site(&["altar", "lab"], Access::Open(viewer()))
}

async fn stream(site: &crate::web::site::Site, last: Option<&str>) -> Events {
    let headers: Vec<(&str, &str)> = last.map(|id| ("last-event-id", id)).into_iter().collect();
    Events::open(&site.router, "/api/stream", &headers).await
}

/// every message until the stream stays quiet for [QUIET]
async fn settled(events: &mut Events) -> Vec<Message> {
    let mut read = Vec::new();
    while let Some(message) = events.within(QUIET).await {
        read.push(message);
    }
    read
}

fn done(message: &Message) -> Option<u64> {
    (message.event == "progress").then(|| message.data["progress"]["builds"]["done"].as_u64())?
}

#[tokio::test(start_paused = true)]
async fn a_browser_reads_a_snapshot_then_each_change_in_seq_order() {
    let site = open();
    let mut events = stream(&site, None).await;

    let snapshot = events.next().await.unwrap();
    assert_eq!(snapshot.event, "snapshot");
    assert_eq!(snapshot.data["kind"], "snapshot");
    assert_eq!(snapshot.data["schema"], 1);
    assert_eq!(snapshot.data["served_by"], "lab");
    assert_eq!(snapshot.data["viewer"]["login"], "r3dlust@github");
    let machines = snapshot.data["machines"].as_array().unwrap();
    assert_eq!(machines.len(), 2);
    assert_eq!(machines[0]["host"], "altar");
    assert_eq!(machines[1]["link"]["kind"], "connecting");
    assert!(machines[1]["state"].is_null());

    link(&site.inputs, NOW.0, "lab", Link::Connected).await;
    view(&site.inputs, NOW.0, building(10)).await;
    let read = settled(&mut events).await;

    let seqs: Vec<u64> = read.iter().map(Message::seq).collect();
    assert!(
        seqs.windows(2).all(|pair| pair[0] <= pair[1]),
        "out of order: {seqs:?}"
    );
    assert!(seqs[0] >= snapshot.seq());
    let kinds: Vec<&str> = read.iter().map(|message| message.event.as_str()).collect();
    assert!(kinds.contains(&"deployment"), "{kinds:?}");
    let lab = read
        .iter()
        .rfind(|message| message.event == "machine")
        .unwrap();
    assert_eq!(lab.data["machine"]["link"]["kind"], "connected");
    assert_eq!(lab.data["machine"]["state"]["running"]["id"], 61);
    assert_eq!(read.iter().filter_map(done).collect::<Vec<_>>(), [10]);
}

#[tokio::test(start_paused = true)]
async fn a_browser_resuming_in_the_same_process_gets_only_what_changed() {
    let site = open();
    let mut events = stream(&site, None).await;
    events.next().await.unwrap();
    link(&site.inputs, NOW.0, "lab", Link::Connected).await;
    view(&site.inputs, NOW.0, building(10)).await;
    settled(&mut events).await;
    link(&site.inputs, NOW.0 + 1_000, "altar", Link::Connecting).await;
    view(&site.inputs, NOW.0 + 1_000, machine("altar")).await;
    let last = settled(&mut events).await.pop().unwrap();
    drop(events);

    // while the browser is away, altar loses its link and lab builds on
    link(
        &site.inputs,
        NOW.0 + 2_000,
        "altar",
        Link::Unreachable(crate::client::Unreach::Failed("reset".into())),
    )
    .await;
    view(&site.inputs, NOW.0 + 2_000, building(20)).await;
    sleep(QUIET).await;

    let mut resumed = stream(&site, Some(&last.id)).await;
    let read = settled(&mut resumed).await;
    let kinds: Vec<(&str, u64)> = read
        .iter()
        .map(|message| (message.event.as_str(), message.seq()))
        .collect();
    assert_eq!(read.len(), 2, "{kinds:?}");
    assert!(read.iter().all(|message| message.seq() > last.seq()));
    assert_eq!(read[0].data["machine"]["host"], "altar");
    assert_eq!(read[0].data["machine"]["link"]["kind"], "unreachable");
    assert_eq!(done(&read[1]), Some(20));
}

#[tokio::test(start_paused = true)]
async fn a_browser_from_another_process_or_time_starts_over_from_a_snapshot() {
    let site = open();
    let mut events = stream(&site, None).await;
    let snapshot = events.next().await.unwrap();
    let (epoch, _) = snapshot.id.split_once('.').unwrap();

    let another = format!("{:016x}.1", u64::from_str_radix(epoch, 16).unwrap() ^ 1);
    let ahead = format!("{epoch}.{}", snapshot.seq() + 100);
    for last in [another.as_str(), ahead.as_str(), "garbage"] {
        let mut events = stream(&site, Some(last)).await;
        assert_eq!(events.next().await.unwrap().event, "snapshot", "{last}");
    }

    let mut current = stream(&site, Some(&snapshot.id)).await;
    assert!(current.within(QUIET).await.is_none());
}

#[tokio::test(start_paused = true)]
async fn a_build_s_numbers_go_out_at_most_every_quarter_second() {
    let site = open();
    let mut events = stream(&site, None).await;
    events.next().await.unwrap();
    link(&site.inputs, NOW.0, "lab", Link::Connected).await;

    let inputs = site.inputs.clone();
    tokio::spawn(async move {
        for done in 0..=200 {
            view(&inputs, NOW.0, building(done)).await;
            sleep(Duration::from_millis(10)).await;
        }
    });

    let mut sent = Vec::new();
    loop {
        let message = events.next().await.unwrap();
        if let Some(done) = done(&message) {
            sent.push((Instant::now(), done));
            if done == 200 {
                break;
            }
        }
    }

    for pair in sent.windows(2) {
        let gap = pair[1].0 - pair[0].0;
        assert!(
            gap >= PROGRESS,
            "{gap:?} between {:?}",
            (pair[0].1, pair[1].1)
        );
    }
    assert!(sent.len() <= 2_000 / 250 + 2, "{} updates", sent.len());
}

#[tokio::test(start_paused = true)]
async fn a_change_to_a_machine_never_waits_on_a_build_s_numbers() {
    let site = open();
    let mut events = stream(&site, None).await;
    events.next().await.unwrap();
    let start = Instant::now();

    // the snapshot carried numbers just now, so these are held back
    view(&site.inputs, NOW.0, building(1)).await;
    sleep(Duration::from_millis(60)).await;
    let changed = Instant::now();
    link(&site.inputs, NOW.0, "altar", Link::Connected).await;

    let mut arrivals = Vec::new();
    while !arrivals
        .iter()
        .any(|(_, message): &(Instant, Message)| done(message).is_some())
    {
        let message = events.next().await.unwrap();
        arrivals.push((Instant::now(), message));
    }
    let (altar, _) = arrivals
        .iter()
        .find(|(_, message)| message.data["machine"]["host"] == "altar")
        .unwrap();
    let (numbers, _) = arrivals
        .iter()
        .find(|(_, message)| done(message) == Some(1))
        .unwrap();
    assert!(
        *altar - changed <= Duration::from_millis(60),
        "{arrivals:?}"
    );
    assert!(*numbers - start >= PROGRESS, "{arrivals:?}");
}

#[tokio::test(start_paused = true)]
async fn a_stalled_browser_gets_the_latest_values_not_a_backlog() {
    let site = open();
    let mut events = stream(&site, None).await;
    events.next().await.unwrap();
    link(&site.inputs, NOW.0, "lab", Link::Connected).await;

    // the browser reads nothing while lab builds 300 derivations
    for done in 0..=300 {
        view(&site.inputs, NOW.0, building(done)).await;
        sleep(Duration::from_millis(10)).await;
    }
    sleep(QUIET).await;

    let read = events.until(|message| done(message) == Some(300)).await;
    let progress: Vec<u64> = read.iter().filter_map(done).collect();
    assert!(progress.len() <= 3, "a backlog: {progress:?}");
    assert!(events.within(QUIET).await.is_none());
}

#[tokio::test(start_paused = true)]
async fn an_idle_stream_hears_a_heartbeat() {
    let site = open();
    let mut events = stream(&site, None).await;
    events.next().await.unwrap();

    assert!(events.within(Duration::from_secs(16)).await.is_none());
    assert_eq!(events.comments, 1);
}
