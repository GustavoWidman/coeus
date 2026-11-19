use std::{path::PathBuf, sync::Arc};

use axum::http::StatusCode;
use gjallar::Receiver;
use serde_json::{Value, json};

use super::{
    session::Browser,
    sse::{body, signed, start},
};
use crate::{
    common::model::Timestamp,
    config::{Key, WebPushSettings},
    testing,
    web::{
        fixture::{Fixture, PUSH_KEY, PUSH_SUBSCRIBED, Scenario},
        push::{Push, endpoint_hash},
        session::Sessions,
        site::Access,
    },
};

const KEY: &str =
    "BBgqG0VbQAPnBlmSzWD_uRkfTJJ7rNkfZc3HbxlUfhd8DBhqYt_DqIxDFK71gxU01Umj-1CqpRCOP0NQP_4183g";

/// `/api/push` as a signed-in browser reads it from a site answering `push`
async fn read(push: Option<Push>) -> (StatusCode, String) {
    let sessions = Arc::new(Sessions::new(vec![Key::generate()], 7).unwrap());
    let access = Access::Sessions {
        sessions: sessions.clone(),
        signin: Vec::new(),
    };
    let site = start(&["lab"], access, None, push);
    let browser = Browser::new();
    let token = sessions
        .issue(
            "r3dlust@github".into(),
            "iphone".into(),
            browser.public(),
            Timestamp::now(),
        )
        .token;
    let response = signed(&site.router, &browser, &token, "/api/push").await;
    (response.status(), body(response).await)
}

fn live(subscriptions_file: PathBuf) -> Push {
    Push::Live(WebPushSettings {
        public_key: KEY.into(),
        subscriptions_file,
        edit: Some("agenix -e push/subscriptions.age".into()),
    })
}

fn entry(name: &str, endpoint: &str) -> Value {
    testing::push::entry(name, &Receiver::generate().unwrap(), endpoint)
}

#[test]
fn an_endpoint_is_named_by_the_start_of_its_hash() {
    // the app hashes its own endpoint the same way (web/src/lib/push/state.test.ts)
    assert_eq!(endpoint_hash(PUSH_SUBSCRIBED), "4029f3734d796cf1");
    assert_eq!(
        endpoint_hash("https://fcm.googleapis.com/fcm/send/abc"),
        "4e9bdab8bbe7189c"
    );
}

#[tokio::test]
async fn the_app_gets_the_key_and_the_hashes_of_the_list_it_reads_each_time() {
    let dir = tempfile::tempdir().unwrap();
    let list = dir.path().join("subscriptions.json");
    let iphone = "https://web.push.apple.com/QGxiphone";
    let mac = "https://web.push.apple.com/QGxmac";
    std::fs::write(&list, json!([entry("iphone", iphone)]).to_string()).unwrap();
    let push = live(list.clone());

    let view = push.view().await.unwrap();
    assert_eq!(view.key, KEY);
    assert_eq!(view.subscribed, [endpoint_hash(iphone)]);

    // an edit shows without a restart
    std::fs::write(
        &list,
        json!([entry("iphone", iphone), entry("mac", mac)]).to_string(),
    )
    .unwrap();
    let (status, text) = read(Some(push)).await;
    assert_eq!(status, StatusCode::OK);
    let answer: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(
        answer,
        json!({
            "key": KEY,
            "subscribed": [endpoint_hash(iphone), endpoint_hash(mac)],
            "edit": "agenix -e push/subscriptions.age",
        })
    );
    assert!(!text.contains("QGx"), "no endpoint leaves the server");
    assert!(!text.contains("p256dh"), "no key leaves the server");
}

#[tokio::test]
async fn without_push_the_app_is_told_there_is_none() {
    let (status, text) = read(None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(text, "");
}

#[tokio::test]
async fn a_list_that_cannot_be_read_is_an_error_not_an_empty_list() {
    let dir = tempfile::tempdir().unwrap();
    let missing = live(dir.path().join("missing.json"));
    let (status, text) = read(Some(missing)).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(!text.contains(&*dir.path().to_string_lossy()), "{text}");

    let broken = dir.path().join("broken.json");
    std::fs::write(&broken, "[{\"name\":\"iphone\"}]").unwrap();
    let (status, _) = read(Some(live(broken))).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn the_simulated_fleet_sends_to_one_known_endpoint() {
    let fixture = Fixture::new(Scenario::Instrument, 0.0);
    let router = fixture.site(None, "http://127.0.0.1:7979", true).unwrap();
    let response = super::sse::get(&router, "/api/push", &[]).await;
    assert_eq!(response.status(), StatusCode::OK);
    let answer: Value = serde_json::from_str(&body(response).await).unwrap();
    assert_eq!(answer["key"], PUSH_KEY);
    assert_eq!(
        answer["subscribed"],
        json!([endpoint_hash(PUSH_SUBSCRIBED)])
    );
}
