//! a browser's side of the site, in process

use std::time::Duration;

use axum::{
    Router,
    body::Body,
    http::{Method, Request, Response, StatusCode},
};
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

use super::session::Browser;
use crate::{
    client::{FleetEvent, Link},
    common::model::{Hostname, MachineView, Timestamp},
    config::{ClientSettings, Key},
    web::{
        fleet::Input,
        site::{Access, Settings, Site},
        source::Source,
        view::Viewer,
    },
};

pub fn viewer() -> Viewer {
    Viewer {
        login: "r3dlust@github".into(),
        device: "iphone".into(),
        expires_at: Timestamp(1_900_000_000_000),
    }
}

/// a site over `hosts` whose machines say only what a test hands it
pub fn site(hosts: &[&str], access: Access) -> Site {
    serving(hosts, access, None)
}

pub fn serving(hosts: &[&str], access: Access, web_dir: Option<std::path::PathBuf>) -> Site {
    let settings = ClientSettings::default();
    let names: Vec<String> = hosts.iter().map(|name| (*name).to_owned()).collect();
    let targets = crate::client::Targets::select(&names, &settings).unwrap();
    // never fed: the test hands the aggregator what the machines say
    let fleet = crate::client::Fleet::new(settings, Key::generate(), targets);
    Site::start(
        fleet.hosts().to_vec(),
        Source::Live(fleet),
        Settings {
            web_dir,
            served_by: "lab".parse().unwrap(),
            access,
        },
    )
}

pub async fn view(inputs: &Inputs, at: i64, view: MachineView) {
    let host = view.host.hostname.clone();
    let event = FleetEvent::View {
        host,
        view: Box::new(view),
    };
    send(inputs, at, event).await;
}

pub async fn link(inputs: &Inputs, at: i64, host: &str, link: Link) {
    let host: Hostname = host.parse().unwrap();
    send(inputs, at, FleetEvent::Link { host, link }).await;
}

async fn send(inputs: &Inputs, at: i64, event: FleetEvent) {
    let input = Input::Fleet {
        at: Timestamp(at),
        event,
    };
    inputs.send(input).await.unwrap();
}

pub type Inputs = tokio::sync::mpsc::Sender<Input>;

pub async fn get(router: &Router, path: &str, headers: &[(&str, &str)]) -> Response<Body> {
    let mut request = Request::get(path);
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    router
        .clone()
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap()
}

pub async fn body(response: Response<Body>) -> String {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8(bytes.to_vec()).unwrap()
}

/// one server-sent event
#[derive(Debug)]
pub struct Message {
    pub id: String,
    pub event: String,
    pub data: Value,
}

impl Message {
    /// the seq of a fleet message's id
    pub fn seq(&self) -> u64 {
        self.id.split_once('.').unwrap().1.parse().unwrap()
    }
}

/// what a browser reads of an event stream
pub struct Events {
    body: Body,
    buffer: String,
    /// comments read so far
    pub comments: usize,
}

/// `GET path` as `browser` proves it for its session `token`
pub async fn signed(router: &Router, browser: &Browser, token: &str, path: &str) -> Response<Body> {
    let mut request = Request::get(path).body(Body::empty()).unwrap();
    *request.headers_mut() = browser.headers(&Method::GET, path, token, Timestamp::now());
    router.clone().oneshot(request).await.unwrap()
}

impl Events {
    pub async fn open(router: &Router, path: &str, headers: &[(&str, &str)]) -> Self {
        Self::of(get(router, path, headers).await, path)
    }

    /// the stream `browser` reads at `path` with its session `token`
    pub async fn signed(router: &Router, browser: &Browser, token: &str, path: &str) -> Self {
        Self::of(signed(router, browser, token, path).await, path)
    }

    fn of(response: Response<Body>, path: &str) -> Self {
        assert_eq!(response.status(), StatusCode::OK, "{path}");
        assert_eq!(
            response.headers()["content-type"],
            "text/event-stream",
            "{path}"
        );
        Self {
            body: response.into_body(),
            buffer: String::new(),
            comments: 0,
        }
    }

    /// the next message, `None` once the stream ends
    pub async fn next(&mut self) -> Option<Message> {
        loop {
            if let Some(end) = self.buffer.find("\n\n") {
                let raw: String = self.buffer.drain(..end + 2).collect();
                if raw.starts_with(':') {
                    self.comments += 1;
                    continue;
                }
                return Some(parse(&raw));
            }
            let frame = self.body.frame().await?.unwrap();
            let data = frame.into_data().unwrap();
            self.buffer.push_str(std::str::from_utf8(&data).unwrap());
        }
    }

    /// the next message if one comes within `wait`
    pub async fn within(&mut self, wait: Duration) -> Option<Message> {
        tokio::time::timeout(wait, self.next()).await.ok().flatten()
    }

    /// the messages until one passes `last`
    pub async fn until(&mut self, last: impl Fn(&Message) -> bool) -> Vec<Message> {
        let mut read = Vec::new();
        loop {
            let message = self.next().await.expect("the stream ended");
            let done = last(&message);
            read.push(message);
            if done {
                return read;
            }
        }
    }
}

fn parse(raw: &str) -> Message {
    let field = |name: &str| {
        raw.lines()
            .find_map(|line| line.strip_prefix(name))
            .unwrap_or_else(|| panic!("no {name} in {raw:?}"))
            .to_owned()
    };
    Message {
        id: raw
            .lines()
            .find_map(|line| line.strip_prefix("id: "))
            .unwrap_or_default()
            .to_owned(),
        event: field("event: "),
        data: serde_json::from_str(&field("data: ")).unwrap(),
    }
}
