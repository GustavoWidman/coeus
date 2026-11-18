//! the simulated fleet, read through the real site

use std::{sync::Arc, time::Duration};

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
};
use serde_json::Value;
use tower::ServiceExt;

use super::{
    session::Browser,
    signin,
    sse::{Events, Message, body, get},
};
use crate::{
    common::model::Timestamp,
    web::{
        fixture::{Fixture, START, Scenario},
        signin::Caller,
    },
};

const ORIGIN: &str = "http://127.0.0.1:7979";

const MINUTE: i64 = 60_000;

/// the simulated fleet's site, its clock held at 0, open to anyone
fn simulated() -> (Arc<Fixture>, Router) {
    let fixture = Fixture::new(Scenario::Instrument, 0.0);
    let router = fixture.site(None, ORIGIN, true).unwrap();
    (fixture, router)
}

async fn seek(router: &Router, t: f64) -> Value {
    let request = Request::post(format!("/fixture/seek?t={t}"))
        .body(Body::empty())
        .unwrap();
    let response = router.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    serde_json::from_str(&body(response).await).unwrap()
}

/// the fleet as a browser first sees it
async fn fleet_at(router: &Router) -> Value {
    let mut events = Events::open(router, "/api/stream", &[]).await;
    events.next().await.unwrap().data
}

fn machine<'a>(snapshot: &'a Value, host: &str) -> &'a Value {
    snapshot["machines"]
        .as_array()
        .unwrap()
        .iter()
        .find(|machine| machine["host"] == host)
        .unwrap()
}

/// the deployment of the rollout, by its run
fn rollout_of(snapshot: &Value) -> &Value {
    snapshot["deployments"]
        .as_array()
        .unwrap()
        .iter()
        .find(|deployment| deployment["origin"]["run_id"] == 1184)
        .expect("the rollout shows")
}

fn verdicts(deployment: &Value) -> Vec<(String, String)> {
    deployment["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            (
                row["host"].as_str().unwrap().to_owned(),
                row["verdict"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

/// a snapshot as the timeline, not the path to it, decides: what machines
/// say, without when each was last heard or the diffs read from logs since
fn moment(mut snapshot: Value) -> Value {
    for machine in snapshot["machines"].as_array_mut().unwrap() {
        if let Some(state) = machine["state"].as_object_mut() {
            state.remove("seen_at");
        }
    }
    for deployment in snapshot["deployments"].as_array_mut().unwrap() {
        for row in deployment["rows"].as_array_mut().unwrap() {
            row.as_object_mut().unwrap().remove("diff");
        }
    }
    snapshot
}

/// the rollout once the diffs of its finished jobs were read from their logs
async fn summed(router: &Router) -> Value {
    for _ in 0..100 {
        let snapshot = fleet_at(router).await;
        let rollout = rollout_of(&snapshot).clone();
        let read = rollout["rows"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|row| row["verdict"] == "done")
            .all(|row| row["diff"]["changes"].is_u64());
        if read {
            return rollout;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("the rollout's diffs were never read");
}

#[tokio::test]
async fn the_story_starts_mid_rollout() {
    let (_, router) = simulated();
    let status = seek(&router, 0.0).await;
    assert_eq!(status["now"], START);
    assert_eq!(status["speed"], 0.0);
    let snapshot = fleet_at(&router).await;

    let lab = &machine(&snapshot, "lab")["state"];
    assert_eq!(lab["running"]["id"], 61);
    assert_eq!(lab["running"]["state"]["phase"], "building");
    assert_eq!(lab["next"]["id"], 62);
    let progress = &snapshot["progress"][0];
    assert_eq!(progress["host"], "lab");
    assert_eq!(progress["builds"]["done"], 112);
    assert_eq!(progress["active"].as_array().unwrap().len(), 4);

    let laptop = machine(&snapshot, "laptop-mac");
    assert_eq!(laptop["link"]["kind"], "unreachable");
    assert_eq!(laptop["link"]["reason"]["kind"], "offline");
    assert_eq!(laptop["link"]["reason"]["last_seen"], START - 192 * MINUTE);
    assert_eq!(laptop["state"]["seen_at"], START - 192 * MINUTE);
    assert_eq!(laptop["state"]["recent"].as_array().unwrap().len(), 4);

    let desktop = &machine(&snapshot, "desktop-nixos")["state"];
    assert_eq!(desktop["recent"][0]["state"]["outcome"]["kind"], "failed");
    assert_eq!(
        desktop["recent"][0]["state"]["outcome"]["phase"],
        "building"
    );
    assert_eq!(
        machine(&snapshot, "oracle-1")["state"]["reboot_required"],
        true
    );
    assert_eq!(
        machine(&snapshot, "oracle-2")["state"]["pending_boot"],
        true
    );

    let outcomes: std::collections::BTreeSet<&str> = snapshot["machines"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|machine| machine["state"]["recent"].as_array().unwrap())
        .map(|job| job["state"]["outcome"]["kind"].as_str().unwrap())
        .collect();
    assert_eq!(
        outcomes.into_iter().collect::<Vec<_>>(),
        [
            "cancelled",
            "failed",
            "interrupted",
            "rolled_back",
            "succeeded",
            "superseded",
            "up_to_date"
        ]
    );

    let rollout = rollout_of(&snapshot);
    let waves: Vec<&Value> = rollout["waves"]
        .as_array()
        .unwrap()
        .iter()
        .map(|wave| &wave["hosts"])
        .collect();
    assert_eq!(
        waves,
        [
            &serde_json::json!(["oracle-1"]),
            &serde_json::json!(["desktop-nixos", "lab", "oracle-2"])
        ]
    );
    assert_eq!(
        verdicts(rollout),
        [
            ("oracle-1".into(), "done".into()),
            ("desktop-nixos".into(), "failed".into()),
            ("lab".into(), "running".into()),
            ("oracle-2".into(), "done".into()),
            ("altar".into(), "pending".into()),
            ("laptop-mac".into(), "pending".into()),
        ]
    );
}

#[tokio::test]
async fn the_rollout_finishes_and_the_laptop_wakes_to_catch_up() {
    let (_, router) = simulated();
    let period = seek(&router, 0.0).await["period"].as_f64().unwrap();

    seek(&router, 45.0).await;
    let snapshot = fleet_at(&router).await;
    assert_eq!(machine(&snapshot, "altar")["state"]["running"]["id"], 43);
    assert_eq!(machine(&snapshot, "lab")["state"]["recent"][0]["id"], 61);
    let rollout = rollout_of(&snapshot).clone();
    assert_eq!(rollout["waves"].as_array().unwrap().len(), 3);
    let lab = rollout["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["host"] == "lab")
        .unwrap()
        .clone();
    assert_eq!(lab["verdict"], "done");

    seek(&router, 80.0).await;
    let snapshot = fleet_at(&router).await;
    let laptop = machine(&snapshot, "laptop-mac");
    assert_eq!(laptop["link"]["kind"], "connected");
    assert_eq!(laptop["state"]["running"]["origin"]["kind"], "follow");
    assert_eq!(machine(&snapshot, "lab")["state"]["follow"]["paused"], true);
    assert!(verdicts(rollout_of(&snapshot)).contains(&("laptop-mac".into(), "catching_up".into())));

    seek(&router, period - 1.0).await;
    let rollout = summed(&router).await;
    assert!(verdicts(&rollout).contains(&("laptop-mac".into(), "caught_up".into())));
    let altar = rollout["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["host"] == "altar")
        .unwrap();
    assert_eq!(altar["diff"]["changes"], 5);

    seek(&router, period).await;
    let snapshot = fleet_at(&router).await;
    assert_eq!(machine(&snapshot, "lab")["state"]["running"]["id"], 61);
}

#[tokio::test]
async fn a_moment_looks_the_same_however_the_clock_reached_it() {
    let (_, router) = simulated();
    seek(&router, 50.0).await;
    let direct = moment(fleet_at(&router).await);

    seek(&router, 10.0).await;
    fleet_at(&router).await;
    seek(&router, 50.0).await;
    let again = moment(fleet_at(&router).await);
    assert_eq!(direct["machines"], again["machines"]);
    assert_eq!(direct["progress"], again["progress"]);
    assert_eq!(direct["deployments"], again["deployments"]);

    let (_, elsewhere) = simulated();
    seek(&elsewhere, 50.0).await;
    let fresh = moment(fleet_at(&elsewhere).await);
    assert_eq!(direct["machines"], fresh["machines"]);
}

fn texts(messages: &[Message]) -> Vec<String> {
    messages
        .iter()
        .filter(|message| message.event == "lines")
        .flat_map(|message| message.data["lines"].as_array().unwrap().clone())
        .map(|line| line["text"].as_str().unwrap().to_owned())
        .collect()
}

#[tokio::test]
async fn a_simulated_job_s_log_grows_with_the_clock_and_resumes_by_offset() {
    let (_, router) = simulated();
    seek(&router, 0.0).await;
    let path = "/api/machines/lab/jobs/61/log";

    let mut log = Events::open(&router, path, &[]).await;
    assert_eq!(log.next().await.unwrap().event, "started");
    let first = log.until(|message| message.event == "lines").await;
    let lines = texts(&first);
    assert!(lines.iter().any(|text| text == "building"), "{lines:?}");
    let offset = first.last().unwrap().id.clone();
    assert!(log.within(Duration::from_millis(300)).await.is_none());

    seek(&router, 20.0).await;
    let tail = texts(&[log.next().await.unwrap()]);
    assert!(!tail.is_empty());
    drop(log);

    seek(&router, 40.0).await;
    let mut resumed = Events::open(&router, path, &[("last-event-id", &offset)]).await;
    let rest = resumed.until(|message| message.event == "ended").await;
    let rest_lines = texts(&rest);
    assert_eq!(rest_lines.first(), tail.first(), "resumed at {offset}");
    assert!(rest_lines.iter().any(|text| text == "activating"));
    assert_eq!(rest_lines.last().unwrap(), "succeeded");
    assert_eq!(rest.last().unwrap().data["outcome"]["kind"], "succeeded");

    let response = Events::open(&router, "/api/machines/oracle-1/jobs/24/log", &[]).await;
    let mut history = response;
    let whole = history.until(|message| message.event == "ended").await;
    assert!(
        texts(&whole)
            .iter()
            .any(|text| text.starts_with("5 changed"))
    );
}

#[tokio::test]
async fn a_job_the_fleet_never_ran_is_not_found() {
    let (_, router) = simulated();
    let response = router
        .clone()
        .oneshot(
            Request::get("/api/machines/lab/jobs/9999/log")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
}

#[tokio::test]
async fn signing_in_to_the_simulated_fleet_opens_its_stream() {
    let fixture = Fixture::new(Scenario::Instrument, 0.0);
    let router = fixture.site(None, ORIGIN, false).unwrap().layer(
        axum::extract::connect_info::MockConnectInfo(Caller("127.0.0.1:50000".parse().unwrap())),
    );

    let response = get(&router, "/api/stream", &[]).await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let hello: Value =
        serde_json::from_str(&body(get(&router, "/api/hello", &[]).await).await).unwrap();
    assert_eq!(hello["signin"], serde_json::json!([ORIGIN]));

    let browser = Browser::new();
    let request = signin::request(&browser, ORIGIN, Timestamp::now());
    let (status, answer) = signin::post(&router, &request).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    let answer: Value = serde_json::from_str(&answer).unwrap();
    assert_eq!(answer["viewer"]["login"], Fixture::viewer().login);
    let token = answer["token"].as_str().unwrap();

    let mut events = Events::signed(&router, &browser, token, "/api/stream").await;
    let snapshot = events.next().await.unwrap();
    assert_eq!(snapshot.event, "snapshot");
    assert_eq!(snapshot.data["viewer"], answer["viewer"]);
    assert_eq!(snapshot.data["machines"].as_array().unwrap().len(), 6);
}
