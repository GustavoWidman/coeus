use axum::http::{StatusCode, header};

use super::sse::{body, get, serving, site, viewer};
use crate::web::site::Access;

const INDEX: &str = "<!doctype html><title>coeus</title>";

#[tokio::test]
async fn nothing_under_api_answers_until_browsers_can_sign_in() {
    let closed = site(&["lab"], Access::Closed);
    for path in [
        "/api/stream",
        "/api/machines/lab/jobs/1/log",
        "/api/machines/lab/jobs/1/diff",
        "/api/anything",
    ] {
        let response = get(&closed.router, path, &[]).await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "{path}");
        assert_eq!(body(response).await, "", "{path}");
    }

    let open = site(&["lab"], Access::Open(viewer()));
    let response = get(&open.router, "/api/anything", &[]).await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let response = get(&open.router, "/api/machines/nowhere/jobs/1/log", &[]).await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
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
    let site = serving(&["lab"], Access::Closed, Some(dir.path().to_owned()));

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
