use std::{
    collections::{BTreeSet, HashMap},
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::Instant,
};

use axum::{
    Router,
    body::Body,
    extract::connect_info::MockConnectInfo,
    http::{Method, Request, StatusCode, header},
};
use serde_json::Value;
use tower::ServiceExt;

use super::{session::Browser, sse::body, whois::CAPABILITY};
use crate::{
    common::model::Timestamp,
    config::Key,
    web::{
        session::{Proof, Sessions},
        signin::{SignIn, Signin},
        whois::{Peer, Policy, Whois},
    },
};

pub const ORIGIN: &str = "https://coeus.example.com";
const PEER: &str = "100.73.0.1:51234";

pub fn phone() -> Peer {
    Peer {
        login: "someone@example.com".into(),
        device: "phone".into(),
        tagged: false,
        capabilities: BTreeSet::from([CAPABILITY.to_owned()]),
    }
}

pub fn policy() -> Policy {
    Policy {
        capability: CAPABILITY.into(),
        logins: Vec::new(),
    }
}

fn signin(sessions: &Arc<Sessions>, whois: Whois, policy: Policy) -> Router {
    Signin {
        sessions: sessions.clone(),
        whois,
        policy,
        origin: ORIGIN.into(),
    }
    .router()
    .unwrap()
    .layer(MockConnectInfo(PEER.parse::<SocketAddr>().unwrap()))
}

pub fn sessions() -> Arc<Sessions> {
    Arc::new(Sessions::new(vec![Key::generate()], 7).unwrap())
}

/// what `browser` sends to sign in to `origin` at `ts`
pub fn request(browser: &Browser, origin: &str, ts: Timestamp) -> SignIn {
    let sig = browser.sign(&Proof::signin(origin, ts));
    SignIn {
        key: browser.jwk(),
        ts,
        sig: base64::Engine::encode(&base64::prelude::BASE64_URL_SAFE_NO_PAD, sig.to_bytes()),
    }
}

pub async fn post(router: &Router, request: &SignIn) -> (StatusCode, String) {
    let request = Request::post("/auth/session")
        .header(header::ORIGIN, ORIGIN)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(serde_json::to_vec(request).unwrap()))
        .unwrap();
    let response = router.clone().oneshot(request).await.unwrap();
    (response.status(), body(response).await)
}

#[tokio::test]
async fn a_granted_device_signs_in_with_its_own_key() {
    let sessions = sessions();
    let router = signin(&sessions, Whois::Fixed(phone()), policy());
    let browser = Browser::new();
    let now = Timestamp::now();

    let (status, answer) = post(&router, &request(&browser, ORIGIN, now)).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    let answer: Value = serde_json::from_str(&answer).unwrap();
    assert_eq!(answer["viewer"]["login"], "someone@example.com");
    assert_eq!(answer["viewer"]["device"], "phone");

    let token = answer["token"].as_str().unwrap();
    let claims = sessions.check(token, now).unwrap();
    assert_eq!(claims.key, browser.public());
    assert_eq!(answer["viewer"]["expires_at"], claims.exp.0);
    assert!(claims.iat.0 >= now.0);
}

#[tokio::test]
async fn a_sign_in_must_be_signed_now_for_this_site_by_its_key() {
    let sessions = sessions();
    let router = signin(&sessions, Whois::Fixed(phone()), policy());
    let browser = Browser::new();
    let now = Timestamp::now();

    let stale = request(&browser, ORIGIN, Timestamp(now.0 - 10 * 60 * 1000));
    let elsewhere = request(&browser, "https://evil.example.com", now);
    let borrowed = SignIn {
        key: Browser::new().jwk(),
        ..request(&browser, ORIGIN, now)
    };
    let mut keyless = request(&browser, ORIGIN, now);
    keyless.key.crv = "P-384".into();

    for request in [stale, elsewhere, borrowed, keyless] {
        let (status, why) = post(&router, &request).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{why}");
    }
}

#[tokio::test]
async fn only_a_users_granted_or_listed_device_signs_in() {
    let sessions = sessions();
    let browser = Browser::new();
    let now = Timestamp::now();
    let ungranted = Peer {
        capabilities: BTreeSet::new(),
        ..phone()
    };
    let tagged = Peer {
        tagged: true,
        ..phone()
    };
    let listed = Policy {
        logins: vec!["someone@example.com".into()],
        ..policy()
    };
    let unknown = Whois::Tailscale(Mutex::new(HashMap::from([(
        PEER.parse::<SocketAddr>().unwrap().ip(),
        (Instant::now(), None),
    )])));

    for (whois, policy, expected) in [
        (
            Whois::Fixed(ungranted.clone()),
            policy(),
            StatusCode::FORBIDDEN,
        ),
        (
            Whois::Fixed(tagged.clone()),
            listed.clone(),
            StatusCode::FORBIDDEN,
        ),
        (unknown, listed.clone(), StatusCode::FORBIDDEN),
        (Whois::Fixed(ungranted), listed, StatusCode::OK),
    ] {
        let router = signin(&sessions, whois, policy);
        let (status, why) = post(&router, &request(&browser, ORIGIN, now)).await;
        assert_eq!(status, expected, "{why}");
    }
}

#[tokio::test]
async fn the_app_may_call_it_from_its_own_origin_alone() {
    let router = signin(&sessions(), Whois::Fixed(phone()), policy());
    let preflight = |origin: &str| {
        Request::builder()
            .method(Method::OPTIONS)
            .uri("/auth/session")
            .header(header::ORIGIN, origin)
            .header(header::ACCESS_CONTROL_REQUEST_METHOD, "POST")
            .header(header::ACCESS_CONTROL_REQUEST_HEADERS, "content-type")
            .header("access-control-request-private-network", "true")
            .body(Body::empty())
            .unwrap()
    };

    let response = router.clone().oneshot(preflight(ORIGIN)).await.unwrap();
    assert!(response.status().is_success());
    let headers = response.headers();
    assert_eq!(headers[header::ACCESS_CONTROL_ALLOW_ORIGIN], ORIGIN);
    assert_eq!(headers["access-control-allow-private-network"], "true");
    let methods = headers[header::ACCESS_CONTROL_ALLOW_METHODS]
        .to_str()
        .unwrap();
    assert!(
        methods.contains("POST") && methods.contains("GET"),
        "{methods}"
    );
    assert_eq!(
        headers[header::ACCESS_CONTROL_ALLOW_HEADERS],
        "content-type"
    );

    let response = router
        .clone()
        .oneshot(preflight("https://evil.example.com"))
        .await
        .unwrap();
    // a browser on another origin sees this one named and refuses
    assert_eq!(
        response.headers()[header::ACCESS_CONTROL_ALLOW_ORIGIN],
        ORIGIN
    );

    let ping = Request::get("/auth/ping")
        .header(header::ORIGIN, ORIGIN)
        .body(Body::empty())
        .unwrap();
    let response = router.oneshot(ping).await.unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert_eq!(
        response.headers()[header::ACCESS_CONTROL_ALLOW_ORIGIN],
        ORIGIN
    );
}
