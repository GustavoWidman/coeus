//! the site cloudflared reaches: the built web app, and the fleet under
//! `/api` for a browser that proves its session on every request. signing
//! in is not a route of this site ([super::signin])

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use axum::{
    Json, Router,
    extract::{FromRequestParts, OriginalUri, Path as Segments, Request, State},
    http::{HeaderMap, HeaderValue, StatusCode, header, request::Parts},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
};
use sha2::{Digest, Sha256};
use tokio::sync::{mpsc, watch};
use tower_http::services::{ServeDir, ServeFile};

use super::{
    diffs::Diffs,
    fleet::{Aggregator, FleetState, Input},
    logs,
    session::Sessions,
    source::Source,
    stream::{self, Cursor, Masthead},
    view::{DiffView, FleetEvent, Hello, LogEvent, SCHEMA, Viewer},
};
use crate::common::model::{Hostname, Timestamp};

/// who may read the fleet
#[derive(Clone)]
pub enum Access {
    /// a browser signed in on the tailnet, at one of `signin`, the sign-in
    /// addresses `/api/hello` offers
    Sessions {
        sessions: Arc<Sessions>,
        signin: Vec<String>,
    },
    /// everyone, as this viewer: a simulated fleet has nothing to hide
    Open(Viewer),
}

/// how the site runs
pub struct Settings {
    /// the built web app; without one only `/api` answers
    pub web_dir: Option<PathBuf>,
    pub served_by: Hostname,
    pub access: Access,
}

/// the site, and where the aggregator takes what the machines say
pub struct Site {
    pub router: Router,
    pub inputs: mpsc::Sender<Input>,
    pub fleet: watch::Receiver<Arc<FleetState>>,
    pub diffs: Arc<Diffs>,
}

struct App {
    fleet: watch::Receiver<Arc<FleetState>>,
    hosts: Vec<Hostname>,
    source: Source,
    diffs: Arc<Diffs>,
    build: String,
    served_by: Hostname,
    access: Access,
}

type Shared = Arc<App>;

impl Site {
    /// starts the aggregator over `hosts` and builds the site around it.
    /// views and links reach it through [Site::inputs]
    pub fn start(hosts: Vec<Hostname>, source: Source, settings: Settings) -> Self {
        let diffs = Arc::new(Diffs::default());
        let aggregator = Aggregator::new(&hosts, diffs.clone());
        let (published, fleet) = watch::channel(Arc::new(aggregator.state().clone()));
        let (inputs, received) = mpsc::channel(256);
        tokio::spawn(aggregator.run(received, published));
        tokio::spawn(
            diffs
                .clone()
                .follow(source.clone(), fleet.clone(), inputs.clone()),
        );

        let app = Arc::new(App {
            fleet: fleet.clone(),
            hosts,
            source,
            diffs: diffs.clone(),
            build: settings.web_dir.as_deref().map_or_else(String::new, build),
            served_by: settings.served_by,
            access: settings.access,
        });

        let api = Router::new()
            .route("/hello", get(hello))
            .route("/stream", get(fleet_stream))
            .route("/machines/{host}/jobs/{id}/log", get(log))
            .route("/machines/{host}/jobs/{id}/diff", get(diff))
            .fallback(|_: Signed| async { StatusCode::NOT_FOUND });
        let mut router = Router::new().nest("/api", api);
        if let Some(web_dir) = &settings.web_dir {
            router = router.merge(web(web_dir));
        }
        let router = router.layer(middleware::from_fn(guard)).with_state(app);

        Self {
            router,
            inputs,
            fleet,
            diffs,
        }
    }
}

/// the web app build, named by its `index.html`, which names every other
/// file by its content
fn build(web_dir: &Path) -> String {
    std::fs::read(web_dir.join("index.html")).map_or_else(
        |_| String::new(),
        |index| {
            Sha256::digest(index)[..6]
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect()
        },
    )
}

/// where the app signs in, and what it is talking to; the one thing under
/// `/api` that answers before signing in, and it carries no fleet data
async fn hello(State(app): State<Shared>) -> Json<Hello> {
    let signin = match &app.access {
        Access::Sessions { signin, .. } => signin.clone(),
        Access::Open(_) => Vec::new(),
    };
    Json(Hello {
        signin,
        build: app.build.clone(),
        schema: SCHEMA,
    })
}

/// a request [Access] lets in: holding one means its session checked and
/// its proof is over this very request
struct Signed {
    viewer: Viewer,
    /// when its session ends, if it does
    expires: Option<Timestamp>,
}

impl FromRequestParts<Shared> for Signed {
    type Rejection = StatusCode;

    async fn from_request_parts(parts: &mut Parts, app: &Shared) -> Result<Self, Self::Rejection> {
        let sessions = match &app.access {
            Access::Open(viewer) => {
                return Ok(Self {
                    viewer: viewer.clone(),
                    expires: None,
                });
            }
            Access::Sessions { sessions, .. } => sessions,
        };
        // the browser signed the path it asked for, `/api` included
        let uri = parts
            .extensions
            .get::<OriginalUri>()
            .map_or(&parts.uri, |original| &original.0);
        let path = uri.path_and_query().map_or("/", |path| path.as_str());
        let claims = sessions
            .authenticate(&parts.method, path, &parts.headers, Timestamp::now())
            .map_err(|_| StatusCode::UNAUTHORIZED)?;
        Ok(Self {
            viewer: Viewer {
                login: claims.sub,
                device: claims.device,
                expires_at: claims.exp,
            },
            expires: Some(claims.exp),
        })
    }
}

async fn fleet_stream(signed: Signed, State(app): State<Shared>, headers: HeaderMap) -> Response {
    let resume = last_event_id(&headers).and_then(|id| id.parse::<Cursor>().ok());
    let masthead = Masthead {
        build: app.build.clone(),
        served_by: app.served_by.clone(),
        viewer: signed.viewer,
    };
    let body = stream::fleet(app.fleet.clone(), resume, masthead);
    events(stream::expiring(body, signed.expires, &FleetEvent::Expired))
}

async fn log(
    signed: Signed,
    State(app): State<Shared>,
    Segments((host, job)): Segments<(String, u64)>,
    headers: HeaderMap,
) -> Response {
    let Some(host) = app.machine(&host) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let from = last_event_id(&headers)
        .and_then(|id| id.parse().ok())
        .unwrap_or(0);

    match logs::follow(&app.source, &host, job, from).await {
        Ok(body) => events(stream::expiring(body, signed.expires, &LogEvent::Expired)),
        Err(error) => (StatusCode::BAD_GATEWAY, format!("{error:#}")).into_response(),
    }
}

async fn diff(
    _: Signed,
    State(app): State<Shared>,
    Segments((host, job)): Segments<(String, u64)>,
) -> Response {
    let Some(host) = app.machine(&host) else {
        return StatusCode::NOT_FOUND.into_response();
    };

    match app.diffs.get(&app.source, &host, job).await {
        Ok(Some(diff)) => Json(DiffView::from(&*diff)).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            format!("job {job} changed no packages"),
        )
            .into_response(),
        Err(error) => (StatusCode::BAD_GATEWAY, format!("{error:#}")).into_response(),
    }
}

impl App {
    fn machine(&self, name: &str) -> Option<Hostname> {
        self.hosts
            .iter()
            .find(|host| host.as_str() == name)
            .cloned()
    }
}

fn last_event_id(headers: &HeaderMap) -> Option<&str> {
    headers.get("last-event-id")?.to_str().ok()
}

fn events(body: axum::body::Body) -> Response {
    (
        [
            (header::CONTENT_TYPE, "text/event-stream"),
            (header::CACHE_CONTROL, "no-cache, no-transform"),
        ],
        body,
    )
        .into_response()
}

/// what every response carries
async fn guard(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    // the page's own policy is a meta in index.html, where frame-ancestors
    // has no effect
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static("frame-ancestors 'none'"),
    );
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    response
}

/// the built app. unknown paths get `index.html` so the app's own routes
/// deep-link, except under `/_app`, where a missing file is a 404. the build
/// wrote a `.br` and `.gz` beside each text file, sent as they are.
fn web(web_dir: &Path) -> Router<Shared> {
    let index = ServeFile::new(web_dir.join("index.html"))
        .precompressed_br()
        .precompressed_gzip();
    let spa = ServeDir::new(web_dir)
        .precompressed_br()
        .precompressed_gzip()
        .fallback(index);
    let assets = ServeDir::new(web_dir.join("_app"))
        .precompressed_br()
        .precompressed_gzip();
    let web_dir: Arc<Path> = web_dir.into();

    Router::new()
        .nest_service("/_app", assets)
        .fallback_service(spa)
        .layer(middleware::from_fn(move |request, next| {
            caching(web_dir.clone(), request, next)
        }))
}

/// hashed build files never change, so browsers keep them a year. the rest,
/// `index.html` and the service worker first, are checked on every load
/// against the ETag the build wrote beside each: in the nix store every file
/// has the same mtime, so a validator built from it would call a new
/// `index.html` of the same size unchanged
async fn caching(web_dir: Arc<Path>, mut request: Request, next: Next) -> Response {
    let path = request.uri().path().to_owned();
    if path.starts_with("/_app/immutable/") {
        let mut response = next.run(request).await;
        if response.status().is_success() {
            response.headers_mut().insert(
                header::CACHE_CONTROL,
                HeaderValue::from_static("public, max-age=31536000, immutable"),
            );
        }
        return response;
    }

    let validator = request.headers_mut().remove(header::IF_NONE_MATCH);
    request.headers_mut().remove(header::IF_MODIFIED_SINCE);
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.remove(header::LAST_MODIFIED);
    headers.remove(header::ETAG);
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    if !response.status().is_success() {
        return response;
    }

    let encoding = response
        .headers()
        .get(header::CONTENT_ENCODING)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let Some(etag) = etag(&web_dir, &path, encoding.as_deref()) else {
        return response;
    };
    if validator.is_some_and(|validator| validator.as_bytes() == etag.as_bytes()) {
        let mut unchanged = StatusCode::NOT_MODIFIED.into_response();
        let headers = unchanged.headers_mut();
        headers.insert(header::ETAG, etag);
        headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
        return unchanged;
    }
    response.headers_mut().insert(header::ETAG, etag);
    response
}

/// the ETag the build wrote for the file `path` serves in `encoding`
fn etag(web_dir: &Path, path: &str, encoding: Option<&str>) -> Option<HeaderValue> {
    let relative = path.trim_start_matches('/');
    if relative.split('/').any(|part| part == ".." || part == ".") {
        return None;
    }
    let mut file = web_dir.join(relative);
    if file.is_dir() {
        file = file.join("index.html");
    }
    if !file.is_file() {
        file = web_dir.join("index.html");
    }
    let suffix = match encoding {
        Some("br") => ".br.etag",
        Some("gzip") => ".gz.etag",
        _ => ".etag",
    };
    let mut sidecar = file.into_os_string();
    sidecar.push(suffix);
    let etag = std::fs::read_to_string(sidecar).ok()?;
    HeaderValue::from_str(etag.trim()).ok()
}
