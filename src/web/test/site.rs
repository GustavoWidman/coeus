use std::{sync::Arc, time::Duration};

use axum::{
    Router,
    body::Body,
    http::{Method, Request, StatusCode, header},
};
use tower::ServiceExt;

use super::{
    session::Browser,
    sse::{Events, body, get, serving, signed, site},
};
use crate::{
    common::model::Timestamp,
    config::Key,
    web::{
        session::{Session, Sessions},
        site::Access,
        view::SCHEMA,
    },
};

const INDEX: &str = "<!doctype html><title>coeus</title>";
const SIGNIN: &str = "https://lab.tail0000.ts.net:45730";
const DAY: i64 = 24 * 60 * 60 * 1000;

fn sessions() -> Arc<Sessions> {
    Arc::new(Sessions::new(vec![Key::generate()], 7).unwrap())
}

fn signed_in(sessions: &Arc<Sessions>) -> Access {
    Access::Sessions {
        sessions: sessions.clone(),
        signin: vec![SIGNIN.into()],
    }
}

/// a session for `browser`, signed in at `at`
fn session(sessions: &Sessions, browser: &Browser, at: Timestamp) -> Session {
    sessions.issue(
        "r3dlust@github".into(),
        "iphone".into(),
        browser.public(),
        at,
    )
}

#[tokio::test]
async fn nothing_under_api_answers_without_a_session() {
    let sessions = sessions();
    let site = site(&["lab"], signed_in(&sessions));
    let browser = Browser::new();
    let token = session(&sessions, &browser, Timestamp::now()).token;
    let paths = [
        "/api/stream",
        "/api/machines/lab/jobs/1/log",
        "/api/machines/lab/jobs/1/diff",
        "/api/anything",
    ];

    for path in paths {
        let response = get(&site.router, path, &[]).await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "{path}");
        assert_eq!(body(response).await, "", "{path}");

        // a token alone, without the key that signs for it
        let bare = format!("Coeus {token}");
        let response = get(&site.router, path, &[("authorization", &bare)]).await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "{path}");
    }

    let thief = Browser::new();
    let response = signed(&site.router, &thief, &token, "/api/stream").await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    // a proof for one path does not open another
    let mut request = Request::get("/api/machines/lab/jobs/1/diff")
        .body(Body::empty())
        .unwrap();
    *request.headers_mut() = browser.headers(&Method::GET, "/api/stream", &token, Timestamp::now());
    let response = site.router.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let rotated = Arc::new(Sessions::new(vec![Key::generate()], 7).unwrap());
    let elsewhere = site_with(&rotated);
    let response = signed(&elsewhere, &browser, &token, "/api/stream").await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let response = signed(&site.router, &browser, &token, "/api/anything").await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let response = signed(
        &site.router,
        &browser,
        &token,
        "/api/machines/nowhere/jobs/1/log",
    )
    .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

fn site_with(sessions: &Arc<Sessions>) -> Router {
    site(&["lab"], signed_in(sessions)).router
}

#[tokio::test]
async fn a_signed_in_browser_reads_the_fleet_as_itself() {
    let sessions = sessions();
    let site = site(&["lab"], signed_in(&sessions));
    let browser = Browser::new();
    let session = session(&sessions, &browser, Timestamp::now());

    let mut events = Events::signed(&site.router, &browser, &session.token, "/api/stream").await;
    let snapshot = events.next().await.unwrap();
    assert_eq!(snapshot.event, "snapshot");
    assert_eq!(snapshot.data["viewer"]["login"], "r3dlust@github");
    assert_eq!(snapshot.data["viewer"]["device"], "iphone");
    assert_eq!(snapshot.data["viewer"]["expires_at"], session.claims.exp.0);
}

#[tokio::test(start_paused = true)]
async fn a_stream_ends_when_its_session_does() {
    let sessions = sessions();
    let site = site(&["lab"], signed_in(&sessions));
    let browser = Browser::new();
    // signed in a week ago, less a minute
    let session = session(
        &sessions,
        &browser,
        Timestamp(Timestamp::now().0 - 7 * DAY + 60_000),
    );

    let mut events = Events::signed(&site.router, &browser, &session.token, "/api/stream").await;
    assert_eq!(events.next().await.unwrap().event, "snapshot");
    let expired = events.within(Duration::from_secs(120)).await.unwrap();
    assert_eq!(expired.event, "expired");
    assert_eq!(expired.id, "");
    assert_eq!(expired.data["kind"], "expired");
    assert!(events.next().await.is_none());
}

#[tokio::test]
async fn hello_answers_before_signing_in_and_tells_nothing_of_the_fleet() {
    let site = site(&["lab"], signed_in(&sessions()));
    let response = get(&site.router, "/api/hello", &[]).await;
    assert_eq!(response.status(), StatusCode::OK);
    let hello: serde_json::Value = serde_json::from_str(&body(response).await).unwrap();
    assert_eq!(
        hello,
        serde_json::json!({ "signin": [SIGNIN], "build": "", "schema": SCHEMA })
    );
}

#[tokio::test]
async fn signing_in_is_no_route_of_the_public_site() {
    let dir = web_dir();
    let sessions = sessions();
    let browser = Browser::new();
    let request = super::signin::request(&browser, super::signin::ORIGIN, Timestamp::now());
    let body_of = || Body::from(serde_json::to_vec(&request).unwrap());

    for web_dir in [None, Some(dir.path().to_owned())] {
        let site = serving(&["lab"], signed_in(&sessions), web_dir.clone());
        for path in ["/auth/session", "/auth/ping", "/api/auth/session"] {
            let post = Request::post(path)
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::ORIGIN, super::signin::ORIGIN)
                .body(body_of())
                .unwrap();
            let response = site.router.clone().oneshot(post).await.unwrap();
            let status = response.status();
            let answer = body(response).await;
            assert!(!answer.contains("v1."), "{path}: {answer}");
            assert!(!status.is_success(), "{path} {web_dir:?}: {status}");

            let response = get(&site.router, path, &[]).await;
            let status = response.status();
            let answer = body(response).await;
            match web_dir {
                // the app's own routes deep-link to index.html
                Some(_) if !path.starts_with("/api") => assert_eq!(answer, INDEX, "{path}"),
                _ => assert!(!status.is_success(), "{path}: {status}"),
            }
        }
    }
}

fn web_dir() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let write = |path: &str, contents: &str| {
        let path = dir.path().join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, contents).unwrap();
    };
    write("index.html", INDEX);
    write("index.html.etag", "\"plain\"");
    write("index.html.br", "brotli bytes");
    write("index.html.br.etag", "\"brotli\"");
    write("service-worker.js", "self.skipWaiting()");
    write("service-worker.js.etag", "\"worker\"");
    write("_app/immutable/entry.abc123.js", "export {}");
    dir
}

#[tokio::test]
async fn the_app_is_served_to_anyone_and_cached_by_whether_it_can_change() {
    let dir = web_dir();
    let site = serving(
        &["lab"],
        signed_in(&sessions()),
        Some(dir.path().to_owned()),
    );

    for path in ["/", "/machines/lab", "/deployments/00ab"] {
        let response = get(&site.router, path, &[]).await;
        assert_eq!(response.status(), StatusCode::OK, "{path}");
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-cache");
        assert_eq!(response.headers()[header::ETAG], "\"plain\"");
        assert_eq!(
            response.headers()[header::CONTENT_SECURITY_POLICY],
            "frame-ancestors 'none'"
        );
        assert_eq!(response.headers()[header::REFERRER_POLICY], "no-referrer");
        assert_eq!(body(response).await, INDEX, "{path}");
    }

    let response = get(&site.router, "/", &[("accept-encoding", "br")]).await;
    assert_eq!(response.headers()[header::CONTENT_ENCODING], "br");
    assert_eq!(response.headers()[header::ETAG], "\"brotli\"");

    let response = get(&site.router, "/", &[("if-none-match", "\"plain\"")]).await;
    assert_eq!(response.status(), StatusCode::NOT_MODIFIED);
    let response = get(&site.router, "/", &[("if-none-match", "\"older\"")]).await;
    assert_eq!(response.status(), StatusCode::OK);

    let response = get(&site.router, "/service-worker.js", &[]).await;
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-cache");
    assert_eq!(response.headers()[header::ETAG], "\"worker\"");

    let response = get(&site.router, "/_app/immutable/entry.abc123.js", &[]).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()[header::CACHE_CONTROL],
        "public, max-age=31536000, immutable"
    );
    let response = get(&site.router, "/_app/immutable/gone.js", &[]).await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}
