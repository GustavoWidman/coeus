use axum::http::{HeaderMap, HeaderValue, Method, header};
use base64::{Engine, prelude::BASE64_URL_SAFE_NO_PAD as B64};
use p256::ecdsa::{Signature, SigningKey, signature::Signer};

use crate::{
    common::model::Timestamp,
    config::Key,
    web::session::{DeviceKey, Jwk, PROOF, Proof, Refused, Session, Sessions},
};

const NOW: Timestamp = Timestamp(1_800_000_000_000);
const DAY: i64 = 24 * 60 * 60 * 1000;
const STREAM: &str = "/api/stream";

/// a browser's key pair, as WebCrypto would hold it
pub struct Browser(SigningKey);

impl Browser {
    pub fn new() -> Self {
        loop {
            if let Ok(key) = SigningKey::from_slice(&rand::random::<[u8; 32]>()) {
                return Self(key);
            }
        }
    }

    pub fn public(&self) -> DeviceKey {
        let point = self.0.verifying_key().to_encoded_point(false);
        DeviceKey::from_sec1(point.as_bytes()).unwrap()
    }

    pub fn jwk(&self) -> Jwk {
        let point = self.0.verifying_key().to_encoded_point(false);
        Jwk {
            kty: "EC".into(),
            crv: "P-256".into(),
            x: B64.encode(point.x().unwrap()),
            y: B64.encode(point.y().unwrap()),
        }
    }

    pub fn sign(&self, message: &[u8]) -> Signature {
        self.0.sign(message)
    }

    /// the headers a request to `path` carries at `ts`
    pub fn headers(&self, method: &Method, path: &str, token: &str, ts: Timestamp) -> HeaderMap {
        let sig = self.sign(&Proof::message(method, path, ts, token));
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_str(&format!("Coeus {token}")).unwrap(),
        );
        headers.insert(
            PROOF,
            HeaderValue::from_str(&Proof { ts, sig }.to_string()).unwrap(),
        );
        headers
    }
}

fn sessions(keys: &[&Key]) -> Sessions {
    Sessions::new(keys.iter().map(|key| (*key).clone()).collect(), 7).unwrap()
}

fn issue(sessions: &Sessions, browser: &Browser) -> Session {
    sessions.issue(
        "r3dlust@github".into(),
        "iphone".into(),
        browser.public(),
        NOW,
    )
}

fn get(sessions: &Sessions, headers: &HeaderMap, at: Timestamp) -> Result<(), Refused> {
    sessions
        .authenticate(&Method::GET, STREAM, headers, at)
        .map(|_| ())
}

#[test]
fn a_session_round_trips_and_lasts_seven_days() {
    let sessions = sessions(&[&Key::generate()]);
    let browser = Browser::new();
    let session = issue(&sessions, &browser);

    assert!(session.token.starts_with("v1."));
    assert_eq!(session.claims.exp, Timestamp(NOW.0 + 7 * DAY));
    assert_eq!(
        sessions.check(&session.token, NOW),
        Ok(session.claims.clone())
    );

    let headers = browser.headers(&Method::GET, STREAM, &session.token, NOW);
    let claims = sessions
        .authenticate(&Method::GET, STREAM, &headers, NOW)
        .unwrap();
    assert_eq!(claims.sub, "r3dlust@github");
    assert_eq!(claims.device, "iphone");
    assert_eq!(claims.key, browser.public());
}

#[test]
fn a_session_ends_when_it_expires() {
    let sessions = sessions(&[&Key::generate()]);
    let browser = Browser::new();
    let session = issue(&sessions, &browser);
    let last = Timestamp(session.claims.exp.0 - 1);

    assert!(sessions.check(&session.token, last).is_ok());
    assert_eq!(
        sessions.check(&session.token, session.claims.exp),
        Err(Refused::Expired)
    );

    let headers = browser.headers(&Method::GET, STREAM, &session.token, session.claims.exp);
    assert_eq!(
        get(&sessions, &headers, session.claims.exp),
        Err(Refused::Expired)
    );
}

#[test]
fn the_first_key_signs_and_every_key_checks() {
    let old = Key::generate();
    let new = Key::generate();
    let browser = Browser::new();
    let before = issue(&sessions(&[&old]), &browser);

    let rotating = sessions(&[&new, &old]);
    assert!(rotating.check(&before.token, NOW).is_ok());
    let after = issue(&rotating, &browser);
    assert_ne!(after.claims.kid, before.claims.kid);
    assert!(sessions(&[&new]).check(&after.token, NOW).is_ok());

    let rotated = sessions(&[&new]);
    assert_eq!(rotated.check(&before.token, NOW), Err(Refused::UnknownKey));
}

#[test]
fn a_session_key_file_holds_one_key_per_line_the_signing_one_first() {
    let first = Key::generate();
    let second = Key::generate();
    let file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(
        file.path(),
        format!("{}\n\n  {}\n", first.to_hex(), second.to_hex()),
    )
    .unwrap();
    let loaded = Sessions::load(file.path(), 7).unwrap();
    let browser = Browser::new();

    let session = issue(&loaded, &browser);
    assert_eq!(
        session.claims.kid,
        issue(&sessions(&[&first]), &browser).claims.kid
    );
    let older = issue(&sessions(&[&second]), &browser);
    assert!(loaded.check(&older.token, NOW).is_ok());

    std::fs::write(file.path(), "\n").unwrap();
    assert!(Sessions::load(file.path(), 7).is_err());
    std::fs::write(file.path(), "not a key\n").unwrap();
    assert!(Sessions::load(file.path(), 7).is_err());
}

#[test]
fn a_proof_holds_for_two_minutes_either_way() {
    let sessions = sessions(&[&Key::generate()]);
    let browser = Browser::new();
    let token = issue(&sessions, &browser).token;
    let at = |offset| Timestamp(NOW.0 + offset);

    for offset in [-120_000, -1, 0, 1, 120_000] {
        let headers = browser.headers(&Method::GET, STREAM, &token, at(offset));
        assert_eq!(get(&sessions, &headers, NOW), Ok(()), "{offset}");
    }
    for offset in [-120_001, 120_001, -DAY] {
        let headers = browser.headers(&Method::GET, STREAM, &token, at(offset));
        assert_eq!(
            get(&sessions, &headers, NOW),
            Err(Refused::Stale),
            "{offset}"
        );
    }
}

#[test]
fn a_proof_by_another_key_is_refused() {
    let sessions = sessions(&[&Key::generate()]);
    let browser = Browser::new();
    let thief = Browser::new();
    let token = issue(&sessions, &browser).token;

    let headers = thief.headers(&Method::GET, STREAM, &token, NOW);
    assert_eq!(get(&sessions, &headers, NOW), Err(Refused::Unproven));
}

#[test]
fn a_proof_is_for_one_request() {
    let sessions = sessions(&[&Key::generate()]);
    let browser = Browser::new();
    let token = issue(&sessions, &browser).token;
    let headers = browser.headers(&Method::GET, "/api/machines/lab/jobs/1/log", &token, NOW);

    for (method, path) in [
        (Method::GET, "/api/machines/lab/jobs/2/log"),
        (Method::GET, "/api/machines/lab/jobs/1/log?x=1"),
        (Method::GET, "/api/machines/lab/jobs/1/diff"),
        (Method::POST, "/api/machines/lab/jobs/1/log"),
    ] {
        assert_eq!(
            sessions.authenticate(&method, path, &headers, NOW),
            Err(Refused::Unproven),
            "{method} {path}"
        );
    }
    assert!(
        sessions
            .authenticate(&Method::GET, "/api/machines/lab/jobs/1/log", &headers, NOW)
            .is_ok()
    );
}

#[test]
fn a_proof_is_for_one_token() {
    let sessions = sessions(&[&Key::generate()]);
    let browser = Browser::new();
    let first = issue(&sessions, &browser).token;
    let second = sessions
        .issue(
            "r3dlust@github".into(),
            "iphone".into(),
            browser.public(),
            Timestamp(NOW.0 + 1),
        )
        .token;
    assert_ne!(first, second);

    let mut headers = browser.headers(&Method::GET, STREAM, &first, NOW);
    headers.insert(
        header::AUTHORIZATION,
        HeaderValue::from_str(&format!("Coeus {second}")).unwrap(),
    );
    assert_eq!(get(&sessions, &headers, NOW), Err(Refused::Unproven));
}

/// `token` with its claims rewritten by `change`, its MAC kept
fn retold(token: &str, change: impl FnOnce(&mut serde_json::Value)) -> String {
    let parts: Vec<&str> = token.split('.').collect();
    let mut claims: serde_json::Value =
        serde_json::from_slice(&B64.decode(parts[1]).unwrap()).unwrap();
    change(&mut claims);
    let claims = B64.encode(serde_json::to_vec(&claims).unwrap());
    format!("{}.{claims}.{}", parts[0], parts[2])
}

#[test]
fn a_tampered_token_is_refused() {
    let sessions = sessions(&[&Key::generate()]);
    let browser = Browser::new();
    let thief = Browser::new();
    let token = issue(&sessions, &browser).token;
    let thiefs_key = serde_json::to_value(thief.public()).unwrap();

    let forged = [
        retold(&token, |claims| claims["sub"] = "someone@else".into()),
        retold(&token, |claims| claims["device"] = "laptop".into()),
        retold(&token, |claims| claims["exp"] = (NOW.0 + 365 * DAY).into()),
        retold(&token, |claims| claims["key"] = thiefs_key.clone()),
        {
            let (signed, mac) = token.rsplit_once('.').unwrap();
            let mut mac = B64.decode(mac).unwrap();
            mac[0] ^= 1;
            format!("{signed}.{}", B64.encode(mac))
        },
    ];
    for token in &forged {
        assert_eq!(sessions.check(token, NOW), Err(Refused::Forged), "{token}");
    }

    assert_eq!(
        sessions.check(
            &retold(&token, |claims| claims["kid"] = "00000000".into()),
            NOW
        ),
        Err(Refused::UnknownKey)
    );
    for malformed in [
        String::new(),
        "v1".into(),
        token.replacen("v1.", "v2.", 1),
        token.rsplit_once('.').unwrap().0.to_owned(),
        retold(&token, |claims| claims["key"] = "AAAA".into()),
        format!("{token}x"),
    ] {
        assert!(
            matches!(
                sessions.check(&malformed, NOW),
                Err(Refused::Malformed | Refused::Forged)
            ),
            "{malformed}"
        );
    }

    // the thief's own proof over a token naming the thief's key
    let stolen = retold(&token, |claims| claims["key"] = thiefs_key);
    let headers = thief.headers(&Method::GET, STREAM, &stolen, NOW);
    assert_eq!(get(&sessions, &headers, NOW), Err(Refused::Forged));
}

#[test]
fn a_request_without_both_headers_is_refused() {
    let sessions = sessions(&[&Key::generate()]);
    let browser = Browser::new();
    let token = issue(&sessions, &browser).token;
    let full = browser.headers(&Method::GET, STREAM, &token, NOW);

    let mut tokenless = full.clone();
    tokenless.remove(header::AUTHORIZATION);
    assert_eq!(get(&sessions, &tokenless, NOW), Err(Refused::Missing));
    let mut unproven = full.clone();
    unproven.remove(PROOF);
    assert_eq!(get(&sessions, &unproven, NOW), Err(Refused::Missing));

    let mut bearer = full.clone();
    bearer.insert(
        header::AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {token}")).unwrap(),
    );
    assert_eq!(get(&sessions, &bearer, NOW), Err(Refused::Malformed));
    let mut garbled = full;
    garbled.insert(PROOF, HeaderValue::from_static("soon.c2lnbmF0dXJl"));
    assert_eq!(get(&sessions, &garbled, NOW), Err(Refused::Malformed));
}

#[test]
fn either_half_of_s_proves_a_request() {
    let sessions = sessions(&[&Key::generate()]);
    let browser = Browser::new();
    let token = issue(&sessions, &browser).token;
    let message = Proof::message(&Method::GET, STREAM, NOW, &token);
    let sig = browser.sign(&message);
    let (r, s) = sig.split_scalars();
    let high = Signature::from_scalars(r, -*s).unwrap();
    assert_ne!(sig, high);

    for sig in [sig, high] {
        let mut headers = browser.headers(&Method::GET, STREAM, &token, NOW);
        headers.insert(
            PROOF,
            HeaderValue::from_str(&Proof { ts: NOW, sig }.to_string()).unwrap(),
        );
        assert_eq!(get(&sessions, &headers, NOW), Ok(()));
    }
}

#[test]
fn a_device_key_is_read_from_a_webcrypto_jwk() {
    let browser = Browser::new();
    assert_eq!(
        DeviceKey::from_jwk(&browser.jwk()).unwrap(),
        browser.public()
    );

    let other_curve = Jwk {
        crv: "P-384".into(),
        ..browser.jwk()
    };
    assert!(DeviceKey::from_jwk(&other_curve).is_err());
    let off_the_curve = Jwk {
        y: browser.jwk().x,
        ..browser.jwk()
    };
    assert!(DeviceKey::from_jwk(&off_the_curve).is_err());
    let short = Jwk {
        x: B64.encode([1; 31]),
        ..browser.jwk()
    };
    assert!(DeviceKey::from_jwk(&short).is_err());
}
