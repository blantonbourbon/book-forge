use std::{
    fs,
    path::PathBuf,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use axum::{
    body::{Body, to_bytes},
    http::{Method, Request, StatusCode, header},
};
use serde_json::{Value, json};
use tower::ServiceExt;

fn fixture_app() -> axum::Router {
    book_forge_server::test_support::fixture_app()
}

fn static_app(static_dir: PathBuf) -> axum::Router {
    book_forge_server::test_support::static_fixture_app(static_dir)
}

fn single_payload(source_url: &str) -> Value {
    json!({
        "sourceUrl": source_url,
        "mode": "single",
        "metadata": {
            "title": "Security Fixture",
            "author": "API Test Author",
            "language": "en",
            "description": "Security validation fixture"
        },
        "options": {
            "includeImages": false
        }
    })
}

fn crawl_payload(source_url: &str, prefix_url: &str) -> Value {
    json!({
        "sourceUrl": source_url,
        "mode": "crawl",
        "metadata": {
            "title": "Security Crawl Fixture",
            "author": "API Test Author",
            "language": "en",
            "description": "Security crawl validation fixture"
        },
        "options": {
            "includeImages": false
        },
        "crawl": {
            "prefixUrl": prefix_url,
            "maxDepth": 1,
            "maxPages": 2,
            "maxTotalBytes": 1048576,
            "maxDurationMillis": 30000
        }
    })
}

async fn json_request(
    app: axum::Router,
    method: Method,
    uri: &str,
    payload: Option<Value>,
) -> (StatusCode, axum::http::HeaderMap, Value) {
    let mut request = Request::builder().method(method).uri(uri);
    let body = if let Some(payload) = payload {
        request = request.header(header::CONTENT_TYPE, "application/json");
        Body::from(serde_json::to_vec(&payload).expect("payload should serialize"))
    } else {
        Body::empty()
    };

    let response = app
        .oneshot(request.body(body).expect("request should build"))
        .await
        .expect("router should respond");
    let status = response.status();
    let headers = response.headers().clone();
    let body = to_bytes(response.into_body(), 1024 * 1024)
        .await
        .expect("body should collect");
    let json = serde_json::from_slice(&body).unwrap_or_else(|error| {
        panic!(
            "expected JSON body for {uri}, got error {error} and body {:?}",
            String::from_utf8_lossy(&body)
        )
    });
    (status, headers, json)
}

async fn body_request(
    app: axum::Router,
    method: Method,
    uri: &str,
) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
    let response = app
        .oneshot(
            Request::builder()
                .method(method)
                .uri(uri)
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("router should respond");
    let status = response.status();
    let headers = response.headers().clone();
    let body = to_bytes(response.into_body(), 1024 * 1024)
        .await
        .expect("body should collect")
        .to_vec();
    (status, headers, body)
}

async fn create_job(app: axum::Router, payload: Value) -> Value {
    let (status, _, body) = json_request(app, Method::POST, "/api/jobs", Some(payload)).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body:#?}");
    body
}

async fn wait_for_terminal(app: axum::Router, id: &str) -> Value {
    for _ in 0..100 {
        let (status, _, body) =
            json_request(app.clone(), Method::GET, &format!("/api/jobs/{id}"), None).await;
        assert_eq!(status, StatusCode::OK, "{body:#?}");
        match body["status"].as_str() {
            Some("completed" | "failed") => return body,
            Some("queued" | "running") => tokio::time::sleep(Duration::from_millis(20)).await,
            other => panic!("unexpected lifecycle status {other:?}: {body:#?}"),
        }
    }
    panic!("job {id} did not reach a terminal state");
}

fn assert_safe_error(body: &Value) {
    let error = &body["error"];
    assert!(
        error["code"].as_str().is_some_and(|code| !code.is_empty()),
        "error code missing: {body:#?}"
    );
    assert!(
        error["message"]
            .as_str()
            .is_some_and(|message| !message.is_empty()),
        "error message missing: {body:#?}"
    );
    assert_no_sensitive_body(body.to_string().as_bytes());
}

fn assert_no_sensitive_body(body: &[u8]) {
    let serialized = String::from_utf8_lossy(body).to_lowercase();
    for forbidden in [
        "/home/",
        "target/debug",
        "backtrace",
        "panic",
        "rustc",
        "[workspace]",
        "book-forge-server",
    ] {
        assert!(
            !serialized.contains(forbidden),
            "unsafe response leaked {forbidden}: {serialized}"
        );
    }
}

#[tokio::test]
async fn unsafe_source_urls_are_rejected_before_job_creation() {
    let app = fixture_app();
    let unsafe_urls = [
        "file:///etc/passwd",
        "ftp://example.test/book.html",
        "gopher://example.test/book",
        "data:text/html,<h1>bad</h1>",
        "javascript:alert(1)",
        "http://127.0.0.1/",
        "http://localhost/",
        "http://10.0.0.1/",
        "http://172.16.0.1/",
        "http://192.168.0.1/",
        "http://169.254.169.254/latest/meta-data/",
        "http://[::1]/",
        "http://[fe80::1]/",
        "http://[fc00::1]/",
        "http://2130706433/",
        "http://0177.0.0.1/",
        "http://0x7f.0.0.1/",
        "http://user@127.0.0.1/",
        "http://localhost./",
        "http://localhost.localdomain/",
    ];

    for source_url in unsafe_urls {
        let (status, _, body) = json_request(
            app.clone(),
            Method::POST,
            "/api/jobs",
            Some(single_payload(source_url)),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "expected rejection for {source_url}: {body:#?}"
        );
        assert_safe_error(&body);
        assert!(
            body.get("id").is_none(),
            "unsafe URL created a job: {body:#?}"
        );
        assert!(
            body["error"]["fields"]
                .as_array()
                .is_some_and(|fields| fields.iter().any(|field| field == "sourceUrl")),
            "sourceUrl field was not named for {source_url}: {body:#?}"
        );
    }
}

#[tokio::test]
async fn unsafe_crawl_prefixes_are_rejected_before_job_creation() {
    let app = fixture_app();
    for prefix_url in [
        "file:///tmp/book/",
        "http://127.0.0.1/crawl/",
        "http://169.254.169.254/crawl/",
        "http://[::1]/crawl/",
    ] {
        let (status, _, body) = json_request(
            app.clone(),
            Method::POST,
            "/api/jobs",
            Some(crawl_payload(
                "https://example.test/crawl-graph/index.html",
                prefix_url,
            )),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "expected rejection for {prefix_url}: {body:#?}"
        );
        assert_safe_error(&body);
        assert!(
            body["error"]["fields"]
                .as_array()
                .is_some_and(|fields| fields.iter().any(|field| field == "crawl.prefixUrl")),
            "crawl.prefixUrl field was not named for {prefix_url}: {body:#?}"
        );
    }
}

#[tokio::test]
async fn redirect_targets_are_revalidated_and_loops_are_bounded() {
    let app = fixture_app();

    let safe_redirect = create_job(
        app.clone(),
        single_payload("https://example.test/redirects/to-safe"),
    )
    .await;
    let safe_terminal =
        wait_for_terminal(app.clone(), safe_redirect["id"].as_str().expect("id")).await;
    assert_eq!(safe_terminal["status"], "completed");

    let private_redirect = create_job(
        app.clone(),
        single_payload("https://example.test/redirects/to-private"),
    )
    .await;
    let private_terminal =
        wait_for_terminal(app.clone(), private_redirect["id"].as_str().expect("id")).await;
    assert_eq!(private_terminal["status"], "failed");
    assert_eq!(private_terminal["errors"][0]["code"], "unsafe_url");
    assert_safe_error(&json!({"error": private_terminal["errors"][0]}));

    let loop_redirect = create_job(
        app.clone(),
        single_payload("https://example.test/redirects/loop-a"),
    )
    .await;
    let loop_terminal = wait_for_terminal(app, loop_redirect["id"].as_str().expect("id")).await;
    assert_eq!(loop_terminal["status"], "failed");
    assert_eq!(
        loop_terminal["errors"][0]["code"],
        "redirect_limit_exceeded"
    );
    assert_safe_error(&json!({"error": loop_terminal["errors"][0]}));
}

#[tokio::test]
async fn static_serving_returns_frontend_assets_and_preserves_api_errors() {
    let static_dir = create_static_dir();
    let app = static_app(static_dir.clone());

    let (status, headers, body) = body_request(app.clone(), Method::GET, "/").await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        headers[header::CONTENT_TYPE]
            .to_str()
            .expect("content type should be text")
            .starts_with("text/html")
    );
    assert!(String::from_utf8_lossy(&body).contains("Book Forge Static Test"));

    let (status, headers, body) = body_request(app.clone(), Method::GET, "/assets/app.css").await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        headers[header::CONTENT_TYPE]
            .to_str()
            .expect("content type should be text")
            .starts_with("text/css")
    );
    assert!(String::from_utf8_lossy(&body).contains("--book-forge-test"));

    let (status, headers, body) = body_request(app.clone(), Method::GET, "/reader/preview").await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        headers[header::CONTENT_TYPE]
            .to_str()
            .expect("content type should be text")
            .starts_with("text/html")
    );
    assert!(String::from_utf8_lossy(&body).contains("Book Forge Static Test"));

    for api_path in ["/api", "/api/", "/api/does-not-exist"] {
        let (status, headers, body) = json_request(app.clone(), Method::GET, api_path, None).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{api_path}: {body:#?}");
        assert!(
            headers[header::CONTENT_TYPE]
                .to_str()
                .expect("content type should be text")
                .starts_with("application/json"),
            "{api_path} did not return a JSON content type"
        );
        assert_safe_error(&body);
        assert_eq!(body["error"]["code"], "api_route_not_found");
    }

    fs::remove_dir_all(static_dir).expect("static fixture should be removed");
}

#[tokio::test]
async fn static_serving_rejects_path_traversal_attempts() {
    let static_dir = create_static_dir();
    let app = static_app(static_dir.clone());

    for path in [
        "/../Cargo.toml",
        "/%2e%2e/Cargo.toml",
        "/assets/%2e%2e/%2e%2e/Cargo.toml",
        "/assets/..%2f..%2fCargo.toml",
    ] {
        let (status, _, body) = body_request(app.clone(), Method::GET, path).await;
        assert!(
            matches!(
                status,
                StatusCode::BAD_REQUEST | StatusCode::FORBIDDEN | StatusCode::NOT_FOUND
            ),
            "unexpected status for {path}: {status}"
        );
        assert_no_sensitive_body(&body);
    }

    fs::remove_dir_all(static_dir).expect("static fixture should be removed");
}

fn create_static_dir() -> PathBuf {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be after epoch")
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "book-forge-static-test-{}-{suffix}",
        std::process::id()
    ));
    fs::create_dir_all(root.join("assets")).expect("static asset dir should be created");
    fs::write(
        root.join("index.html"),
        r#"<!doctype html><html><head><link rel="stylesheet" href="/assets/app.css"></head><body><h1>Book Forge Static Test</h1></body></html>"#,
    )
    .expect("static index should be written");
    fs::write(
        root.join("assets/app.css"),
        ":root { --book-forge-test: 1; }\n",
    )
    .expect("static css should be written");
    root
}
