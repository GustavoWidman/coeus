use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use age::armor::{ArmoredWriter, Format};
use axum::{
    Router,
    body::Bytes,
    extract::{Path as Route, State},
    http::{HeaderMap, StatusCode},
    routing::post,
};
use gjallar::{Receiver, VapidKey};
use std::io::Write;

use super::*;
use crate::{
    bootstrap::secrets::generate::ssh_ed25519,
    cli::commands::push::{line, test_message},
    config::*,
};

fn origin() -> Origin {
    "https://coeus.example.com".parse().unwrap()
}

fn entry(name: &str, receiver: &Receiver, endpoint: &str) -> serde_json::Value {
    let mut json = serde_json::to_value(receiver.subscription(endpoint).unwrap()).unwrap();
    json["name"] = name.into();
    json
}

fn settings(dir: &Path, key: &VapidKey, entries: &[serde_json::Value]) -> PushSettings {
    let vapid_key_file = dir.join("vapid");
    let subscriptions_file = dir.join("subscriptions.json");
    std::fs::write(&vapid_key_file, format!("{}\n", key.to_base64url())).unwrap();
    std::fs::write(&subscriptions_file, serde_json::to_vec(entries).unwrap()).unwrap();
    PushSettings {
        origin: origin(),
        vapid_key_file,
        subscriptions_file,
        identity_file: None,
    }
}

#[test]
fn the_push_section_parses_into_settings() {
    let settings: ClientSettings = toml::from_str(
        r#"
        key_file = "/run/agenix/coeus-key"

        [push]
        origin = "https://coeus.example.com/"
        vapid_key_file = "/run/agenix/coeus-push-vapid"
        subscriptions_file = "/run/agenix/coeus-push-subscriptions"
        "#,
    )
    .unwrap();

    let push = settings.push.unwrap();
    assert_eq!(push.origin.as_str(), "https://coeus.example.com");
    assert_eq!(push.origin.join("/"), "https://coeus.example.com/");
    assert_eq!(
        push.vapid_key_file,
        Path::new("/run/agenix/coeus-push-vapid")
    );
    assert_eq!(push.identity_file, None);
}

#[test]
fn settings_without_a_push_section_send_nothing() {
    let settings: ClientSettings = toml::from_str(r#"key_file = "/k""#).unwrap();
    assert_eq!(settings.push, None);
}

#[test]
fn an_origin_is_https_and_a_host() {
    for good in ["https://coeus.example.com", "https://altar:8443/"] {
        assert!(good.parse::<Origin>().is_ok(), "{good}");
    }
    for bad in [
        "http://coeus.example.com",
        "https://",
        "https://coeus.example.com/app",
        "https://user@coeus.example.com",
        "mailto:me@example.com",
        "coeus.example.com",
    ] {
        assert!(bad.parse::<Origin>().is_err(), "{bad}");
    }
}

#[test]
fn the_list_names_each_device() {
    let receiver = Receiver::generate().unwrap();
    let json = serde_json::to_vec(&[
        entry("iphone", &receiver, "https://web.push.apple.com/a"),
        entry("desktop", &receiver, "https://fcm.googleapis.com/b"),
    ])
    .unwrap();

    let devices = devices(&json).unwrap();
    let names: Vec<&str> = devices.iter().map(|device| device.name.as_str()).collect();
    assert_eq!(names, ["iphone", "desktop"]);
    assert_eq!(
        devices[0].subscription.origin(),
        "https://web.push.apple.com"
    );
}

#[test]
fn a_bad_entry_is_named_and_never_shown() {
    let receiver = Receiver::generate().unwrap();
    let mut broken = entry(
        "iphone",
        &receiver,
        "https://web.push.apple.com/secret-token",
    );
    broken["keys"]["auth"] = "c2hvcnQ".into();
    let error = devices(&serde_json::to_vec(&[broken]).unwrap()).unwrap_err();
    let error = format!("{error:#}");
    assert!(error.contains(r#"subscription "iphone""#), "{error}");
    assert!(error.contains("auth"), "{error}");
    assert!(!error.contains("secret-token"), "{error}");

    let twice = [
        entry("iphone", &receiver, "https://web.push.apple.com/a"),
        entry("iphone", &receiver, "https://web.push.apple.com/b"),
    ];
    let error = devices(&serde_json::to_vec(&twice).unwrap()).unwrap_err();
    assert!(error.to_string().contains("two subscriptions"), "{error}");

    let mut nameless = entry("x", &receiver, "https://web.push.apple.com/a");
    nameless.as_object_mut().unwrap().remove("name");
    assert!(devices(&serde_json::to_vec(&[nameless]).unwrap()).is_err());
    assert!(devices(br#"{"endpoint": "https://web.push.apple.com/a"}"#).is_err());
}

#[test]
fn plain_files_load() {
    let dir = tempfile::tempdir().unwrap();
    let receiver = Receiver::generate().unwrap();
    let key = VapidKey::generate().unwrap();
    let settings = settings(
        dir.path(),
        &key,
        &[entry("iphone", &receiver, "https://web.push.apple.com/a")],
    );

    let push = Push::load(&settings).unwrap();
    assert_eq!(push.devices().len(), 1);
    assert_eq!(push.origin(), &origin());
}

/// `plaintext` encrypted to an ssh key, as agenix would
fn seal(plaintext: &[u8], public: &str, format: Format) -> Vec<u8> {
    let recipient: age::ssh::Recipient = public.parse().unwrap();
    let encryptor =
        age::Encryptor::with_recipients(std::iter::once(&recipient as &dyn age::Recipient))
            .unwrap();
    let mut file = Vec::new();
    let mut writer = encryptor
        .wrap_output(ArmoredWriter::wrap_output(&mut file, format).unwrap())
        .unwrap();
    writer.write_all(plaintext).unwrap();
    writer.finish().and_then(ArmoredWriter::finish).unwrap();
    file
}

#[test]
fn age_files_open_in_memory_with_the_identity() {
    let dir = tempfile::tempdir().unwrap();
    let receiver = Receiver::generate().unwrap();
    let key = VapidKey::generate().unwrap();
    let plain = settings(
        dir.path(),
        &key,
        &[entry("iphone", &receiver, "https://web.push.apple.com/a")],
    );
    let (private, public) = ssh_ed25519("ci").unwrap();
    let identity_file = dir.path().join("identity");
    std::fs::write(&identity_file, private.as_bytes()).unwrap();

    let sealed = |path: &PathBuf, format| {
        let file = path.with_extension("age");
        let plaintext = std::fs::read(path).unwrap();
        std::fs::write(&file, seal(&plaintext, &public, format)).unwrap();
        file
    };
    let settings = PushSettings {
        vapid_key_file: sealed(&plain.vapid_key_file, Format::AsciiArmor),
        subscriptions_file: sealed(&plain.subscriptions_file, Format::Binary),
        identity_file: Some(identity_file),
        ..plain.clone()
    };

    let push = Push::load(&settings).unwrap();
    assert_eq!(push.devices()[0].name, "iphone");

    let (other, _) = ssh_ed25519("other").unwrap();
    std::fs::write(settings.identity_file.as_ref().unwrap(), other.as_bytes()).unwrap();
    let error = Push::load(&settings).err().unwrap();
    assert!(
        format!("{error:#}").contains("failed to decrypt"),
        "{error:#}"
    );

    // an identity set for files that are not encrypted is a mistake, not a fallback
    let mixed = PushSettings {
        identity_file: settings.identity_file.clone(),
        ..plain
    };
    assert!(Push::load(&mixed).is_err());
}

#[test]
fn unusable_files_fail_the_load() {
    let dir = tempfile::tempdir().unwrap();
    let key = VapidKey::generate().unwrap();
    let settings = settings(dir.path(), &key, &[]);

    let missing = PushSettings {
        vapid_key_file: dir.path().join("nothing"),
        ..settings.clone()
    };
    assert!(
        format!("{:#}", Push::load(&missing).err().unwrap()).contains("failed to read"),
        "a missing key"
    );

    std::fs::write(&settings.vapid_key_file, "not a key").unwrap();
    let error = format!("{:#}", Push::load(&settings).err().unwrap());
    assert!(error.contains("VAPID private key"), "{error}");
    assert!(!error.contains("not a key"), "{error}");
}

#[test]
fn each_device_gets_one_line_saying_what_to_do() {
    let ansi = regex_lite::Regex::new("\x1b\\[[0-9;]*m").unwrap();
    let cases = [
        (Delivery::Delivered, "[✓] iphone: delivered"),
        (
            Delivery::Gone { status: 410 },
            "[⚠] iphone: gone (410): the device dropped this subscription, remove it from the list",
        ),
        (
            Delivery::Refused {
                status: 403,
                reason: Some("VapidPkHashMismatch".into()),
            },
            "[✗] iphone: refused (403): VapidPkHashMismatch",
        ),
        (
            Delivery::Refused {
                status: 400,
                reason: None,
            },
            "[✗] iphone: refused (400)",
        ),
        (
            Delivery::RetryLater {
                why: "the push service answered 429".into(),
            },
            "[•] iphone: retry later: the push service answered 429",
        ),
    ];
    for (delivery, expected) in cases {
        assert_eq!(ansi.replace_all(&line("iphone", &delivery), ""), expected);
    }
}

/// what the fake push service saw for each device
#[derive(Default)]
struct Seen {
    payloads: BTreeMap<String, Vec<serde_json::Value>>,
    keys: Vec<String>,
    topics: Vec<String>,
}

/// a push service on loopback that answers per device: `delivered`, `gone`,
/// `rotated` (refused under another VAPID key), `busy` (429 every time)
async fn fake_push_service(receiver: Receiver) -> (String, Arc<Mutex<Seen>>) {
    let seen = Arc::new(Mutex::new(Seen::default()));
    let app = Router::new()
        .route(
            "/push/{device}",
            post(
                async |State((receiver, seen)): State<(Receiver, Arc<Mutex<Seen>>)>,
                       Route(device): Route<String>,
                       headers: HeaderMap,
                       body: Bytes| {
                    let plaintext = receiver.decrypt(&body).unwrap();
                    let authorization = headers["authorization"].to_str().unwrap().to_owned();
                    let mut seen = seen.lock().unwrap();
                    seen.keys
                        .push(authorization.rsplit_once("k=").unwrap().1.to_owned());
                    seen.topics
                        .push(headers["topic"].to_str().unwrap().to_owned());
                    seen.payloads
                        .entry(device.clone())
                        .or_default()
                        .push(serde_json::from_slice(&plaintext).unwrap());
                    match device.as_str() {
                        "gone" => (StatusCode::GONE, String::new()),
                        "rotated" => (
                            StatusCode::FORBIDDEN,
                            r#"{"reason":"VapidPkHashMismatch"}"#.to_owned(),
                        ),
                        "busy" => (StatusCode::TOO_MANY_REQUESTS, String::new()),
                        _ => (StatusCode::CREATED, String::new()),
                    }
                },
            ),
        )
        .with_state((receiver, seen.clone()));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (address, seen)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_test_push_reaches_each_device_and_says_how_it_went() {
    let receiver = Receiver::generate().unwrap();
    let (service, seen) = fake_push_service(receiver.clone()).await;
    let closed = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let unreachable = format!("http://{}/push/nobody", closed.local_addr().unwrap());
    drop(closed);

    let dir = tempfile::tempdir().unwrap();
    let key = VapidKey::generate().unwrap();
    let entries: Vec<_> = ["iphone", "gone", "rotated", "busy"]
        .into_iter()
        .map(|name| entry(name, &receiver, &format!("{service}/push/{name}")))
        .chain([entry("asleep", &receiver, &unreachable)])
        .collect();
    let push = Push::load(&settings(dir.path(), &key, &entries)).unwrap();
    let message = test_message("altar", push.origin()).unwrap();

    let deliveries = tokio::task::spawn_blocking(move || {
        push.send(&message)
            .into_iter()
            .map(|(name, delivery)| (name.to_owned(), delivery))
            .collect::<Vec<_>>()
    })
    .await
    .unwrap();

    let deliveries: BTreeMap<_, _> = deliveries.into_iter().collect();
    assert_eq!(deliveries["iphone"], Delivery::Delivered);
    assert_eq!(deliveries["gone"], Delivery::Gone { status: 410 });
    assert_eq!(
        deliveries["rotated"],
        Delivery::Refused {
            status: 403,
            reason: Some("VapidPkHashMismatch".into())
        }
    );
    assert!(
        matches!(&deliveries["busy"], Delivery::RetryLater { why } if why.contains("429")),
        "{:?}",
        deliveries["busy"]
    );
    assert!(
        matches!(&deliveries["asleep"], Delivery::RetryLater { why } if why.contains("could not reach")),
        "{:?}",
        deliveries["asleep"]
    );

    let seen = seen.lock().unwrap();
    let notification = &seen.payloads["iphone"][0];
    assert_eq!(notification["web_push"], 8030);
    assert_eq!(
        notification["notification"]["title"],
        "coeus test · from altar"
    );
    assert_eq!(
        notification["notification"]["navigate"],
        "https://coeus.example.com/"
    );
    assert_eq!(seen.payloads["busy"].len(), 2, "a 429 is tried once more");
    assert!(seen.keys.iter().all(|k| *k == key.public_key()));
    assert!(
        seen.topics
            .iter()
            .all(|t| *t == seen.topics[0] && t.len() <= 32)
    );
}
