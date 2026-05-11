use std::{
    fs,
    io::ErrorKind,
    net::SocketAddr,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use axum::{
    body::{Body, to_bytes},
    http::{Method, Request, StatusCode, header},
};
use book_forge_epub_inspector::inspect_epub;
use serde_json::{Value, json};
use tower::ServiceExt;

fn fixture_app() -> axum::Router {
    book_forge_server::test_support::fixture_app()
}

fn delayed_fixture_app(delay: Duration) -> axum::Router {
    book_forge_server::test_support::delayed_fixture_app(delay)
}

fn resolved_host_app(domain: &str, addr: SocketAddr) -> axum::Router {
    book_forge_server::test_support::resolved_host_fixture_app(domain, &[addr])
}

fn single_payload(source_url: &str, title: &str) -> Value {
    json!({
        "sourceUrl": source_url,
        "mode": "single",
        "metadata": {
            "title": title,
            "author": "API Test Author",
            "language": "en",
            "description": "API single conversion fixture"
        },
        "options": {
            "includeImages": false,
            "outputTarget": "epub"
        }
    })
}

fn crawl_payload(max_depth: usize, max_pages: usize) -> Value {
    json!({
        "sourceUrl": "https://example.test/crawl-graph/index.html",
        "mode": "crawl",
        "metadata": {
            "title": "API Crawl Fixture",
            "author": "API Test Author",
            "language": "en",
            "description": "API crawl conversion fixture"
        },
        "options": {
            "includeImages": false,
            "outputTarget": "epub"
        },
        "crawl": {
            "prefixUrl": "https://example.test/crawl-graph/",
            "maxDepth": max_depth,
            "maxPages": max_pages,
            "maxTotalBytes": 1048576,
            "maxDurationMillis": 30000
        }
    })
}

fn crawl_images_payload(output_target: &str) -> Value {
    json!({
        "sourceUrl": "https://example.test/images-crawl/index.html",
        "mode": "crawl",
        "metadata": {
            "title": "API Image Crawl Fixture",
            "author": "API Test Author",
            "language": "en",
            "description": "API image crawl conversion fixture"
        },
        "options": {
            "includeImages": true,
            "outputTarget": output_target
        },
        "crawl": {
            "prefixUrl": "https://example.test/images-crawl/",
            "maxDepth": 1,
            "maxPages": 5,
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

async fn binary_request(
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
    let body = to_bytes(response.into_body(), 8 * 1024 * 1024)
        .await
        .expect("body should collect")
        .to_vec();
    (status, headers, body)
}

async fn create_job(app: axum::Router, payload: Value) -> Value {
    let (status, _, body) = json_request(app, Method::POST, "/api/jobs", Some(payload)).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body:#?}");
    assert!(body["id"].as_str().is_some_and(|id| !id.is_empty()));
    assert!(matches!(
        body["status"].as_str(),
        Some("queued" | "running" | "completed")
    ));
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

async fn start_no_content_length_html_server(body_size: usize) -> SocketAddr {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("test TCP listener should bind");
    let addr = listener
        .local_addr()
        .expect("listener should expose address");
    tokio::spawn(async move {
        let Ok((stream, _)) = listener.accept().await else {
            return;
        };
        write_all(
            &stream,
            b"HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nTransfer-Encoding: chunked\r\n\r\n",
        )
        .await;
        let mut body = b"<!doctype html><html><body><h1>Streaming Fixture</h1><p>".to_vec();
        body.extend(std::iter::repeat_n(b'x', body_size));
        body.extend_from_slice(b"</p></body></html>");
        write_all(&stream, format!("{:x}\r\n", body.len()).as_bytes()).await;
        write_all(&stream, &body).await;
        write_all(&stream, b"\r\n").await;
        tokio::time::sleep(Duration::from_secs(2)).await;
    });
    addr
}

async fn write_all(stream: &tokio::net::TcpStream, mut bytes: &[u8]) {
    while !bytes.is_empty() {
        if stream.writable().await.is_err() {
            return;
        }
        match stream.try_write(bytes) {
            Ok(0) => return,
            Ok(written) => bytes = &bytes[written..],
            Err(error) if error.kind() == ErrorKind::WouldBlock => {}
            Err(_) => return,
        }
    }
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
    let serialized = body.to_string().to_lowercase();
    for forbidden in ["/home/", "target/debug", "backtrace", "panic", "rustc"] {
        assert!(
            !serialized.contains(forbidden),
            "unsafe error body leaked {forbidden}: {body:#?}"
        );
    }
}

fn inspect_epub_bytes(bytes: &[u8]) -> book_forge_epub_inspector::InspectionReport {
    let path = std::env::temp_dir().join(format!(
        "book-forge-api-{}.epub",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos()
    ));
    fs::write(&path, bytes).expect("temporary EPUB should be written");
    let report = inspect_epub(&path);
    fs::remove_file(path).expect("temporary EPUB should be removed");
    report
}

#[tokio::test]
async fn health_and_unsupported_methods_return_safe_json() {
    let app = fixture_app();

    let (status, headers, body) = json_request(app.clone(), Method::GET, "/api/health", None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        headers[header::CONTENT_TYPE]
            .to_str()
            .expect("content type should be text")
            .starts_with("application/json")
    );
    assert_eq!(body["status"], "healthy");

    let (status, _, body) = json_request(app.clone(), Method::POST, "/api/health", None).await;
    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
    assert_safe_error(&body);
    assert_eq!(body["error"]["code"], "method_not_allowed");

    let (status, _, body) = json_request(app.clone(), Method::PUT, "/api/jobs", None).await;
    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
    assert_safe_error(&body);

    let (status, _, body) =
        json_request(app, Method::DELETE, "/api/jobs/not-a-uuid/download", None).await;
    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
    assert_safe_error(&body);
}

#[tokio::test]
async fn creates_single_job_reports_status_and_downloads_safe_epub() {
    let app = fixture_app();
    let created = create_job(
        app.clone(),
        single_payload(
            "https://example.test/single-page/index.html",
            "Unsafe / <b>API</b>\r\n Title ..",
        ),
    )
    .await;
    assert_eq!(created["mode"], "single");

    let id = created["id"].as_str().expect("id should be present");
    let terminal = wait_for_terminal(app.clone(), id).await;
    assert_eq!(terminal["id"], id);
    assert_eq!(terminal["status"], "completed");
    assert_eq!(terminal["progress"]["percent"], 100);
    assert_eq!(
        terminal["warnings"]
            .as_array()
            .expect("warnings array")
            .len(),
        0
    );
    assert_eq!(
        terminal["errors"].as_array().expect("errors array").len(),
        0
    );
    assert_eq!(terminal["downloadUrl"], format!("/api/jobs/{id}/download"));

    let (status, headers, bytes) =
        binary_request(app, Method::GET, &format!("/api/jobs/{id}/download")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers[header::CONTENT_TYPE], "application/epub+zip");
    let disposition = headers[header::CONTENT_DISPOSITION]
        .to_str()
        .expect("content disposition should be ascii");
    assert!(disposition.starts_with("attachment; filename=\""));
    assert!(disposition.ends_with(".epub\""));
    for forbidden in ['/', '\\', '\r', '\n'] {
        assert!(!disposition.contains(forbidden));
    }
    assert!(bytes.starts_with(b"PK"));
    assert!(bytes.len() > 512);
    let report = inspect_epub_bytes(&bytes);
    assert!(report.ok, "inspection errors: {:?}", report.errors);
    let package = report.package.expect("package should inspect");
    assert_eq!(package.metadata.title, "Unsafe API Title");
    assert_eq!(package.metadata.author, "API Test Author");
    assert_eq!(package.metadata.language, "en");
    assert_eq!(
        package.metadata.description,
        "API single conversion fixture"
    );
    assert_eq!(package.content_chapters.len(), 1);
    assert_eq!(package.nav_entries.len(), 1);
}

#[tokio::test]
async fn crawl_jobs_surface_progress_and_structured_limit_warnings() {
    let app = fixture_app();
    let created = create_job(app.clone(), crawl_payload(0, 10)).await;
    let id = created["id"].as_str().expect("id should be present");

    let terminal = wait_for_terminal(app, id).await;
    assert_eq!(terminal["status"], "completed");
    assert_eq!(terminal["mode"], "crawl");
    assert_eq!(terminal["progress"]["percent"], 100);
    assert_eq!(terminal["progress"]["pagesFetched"], 1);
    assert_eq!(terminal["progress"]["maxDepth"], 0);
    assert_eq!(terminal["progress"]["maxPages"], 10);
    assert!(
        terminal["warnings"]
            .as_array()
            .expect("warnings array")
            .iter()
            .any(|warning| warning["code"] == "crawl_depth_limit"
                && warning["message"]
                    .as_str()
                    .is_some_and(|message| message.contains("depth")))
    );
}

#[tokio::test]
async fn runtime_limits_cover_slow_fixture_crawl_time_and_streaming_byte_limit() {
    let slow_app = delayed_fixture_app(Duration::from_millis(150));
    let mut slow_payload = crawl_payload(0, 1);
    slow_payload["sourceUrl"] = json!("https://example.test/single-page/index.html");
    slow_payload["crawl"]["prefixUrl"] = json!("https://example.test/single-page/");
    slow_payload["crawl"]["maxDurationMillis"] = json!(50);
    let started = Instant::now();
    let slow_created = create_job(slow_app.clone(), slow_payload).await;
    let slow_terminal = wait_for_terminal(
        slow_app,
        slow_created["id"].as_str().expect("id should be present"),
    )
    .await;
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "slow fixture timeout should terminate promptly"
    );
    assert_eq!(slow_terminal["status"], "failed");
    assert_eq!(slow_terminal["errors"][0]["code"], "fetch_timeout");
    assert_safe_error(&json!({"error": slow_terminal["errors"][0]}));

    let duration_app = delayed_fixture_app(Duration::from_millis(70));
    let mut duration_payload = crawl_payload(2, 10);
    duration_payload["crawl"]["maxDurationMillis"] = json!(120);
    let duration_created = create_job(duration_app.clone(), duration_payload).await;
    let duration_terminal = wait_for_terminal(
        duration_app,
        duration_created["id"]
            .as_str()
            .expect("id should be present"),
    )
    .await;
    assert_eq!(duration_terminal["status"], "completed");
    assert!(
        duration_terminal["warnings"]
            .as_array()
            .expect("warnings array")
            .iter()
            .any(|warning| warning["code"] == "crawl_time_limit"
                && warning["message"]
                    .as_str()
                    .is_some_and(|message| message.contains("time limit"))),
        "crawl duration expiry was not surfaced: {duration_terminal:#?}"
    );

    let stream_domain = "example.test";
    let stream_addr = start_no_content_length_html_server(512).await;
    let stream_app = resolved_host_app(stream_domain, stream_addr);
    let stream_url = format!("http://{stream_domain}:{}/stream.html", stream_addr.port());
    let stream_payload = json!({
        "sourceUrl": stream_url,
        "mode": "crawl",
        "metadata": {
            "title": "Streaming Byte Limit",
            "author": "API Test Author",
            "language": "en",
            "description": "No content length byte limit fixture"
        },
        "options": {
            "includeImages": false,
            "outputTarget": "epub"
        },
        "crawl": {
            "prefixUrl": format!("http://{stream_domain}:{}/", stream_addr.port()),
            "maxDepth": 0,
            "maxPages": 1,
            "maxTotalBytes": 128,
            "maxDurationMillis": 500
        }
    });
    let stream_created = create_job(stream_app.clone(), stream_payload).await;
    let stream_terminal = wait_for_terminal(
        stream_app,
        stream_created["id"].as_str().expect("id should be present"),
    )
    .await;
    assert_eq!(stream_terminal["status"], "failed");
    assert_eq!(stream_terminal["errors"][0]["code"], "response_too_large");
    assert!(
        stream_terminal["errors"][0]["message"]
            .as_str()
            .is_some_and(|message| message.contains("byte limit"))
    );
    assert_safe_error(&json!({"error": stream_terminal["errors"][0]}));
}

#[tokio::test]
async fn single_jobs_include_images_fetches_resources_for_epub_output() {
    let app = fixture_app();
    let mut payload = single_payload(
        "https://example.test/images-crawl/index.html",
        "API Single Images",
    );
    payload["options"]["includeImages"] = json!(true);

    let created = create_job(app.clone(), payload).await;
    let id = created["id"].as_str().expect("id should be present");
    let terminal = wait_for_terminal(app.clone(), id).await;
    assert_eq!(terminal["status"], "completed");
    assert_eq!(terminal["mode"], "single");
    assert_eq!(terminal["summary"]["options"]["includeImages"], true);
    assert!(
        terminal["warnings"]
            .as_array()
            .expect("warnings array")
            .iter()
            .any(|warning| warning["code"] == "image_fetch_failed"
                && warning["affected"] == "https://example.test/images/missing.png"),
        "single-page missing-image warning was not surfaced: {terminal:#?}"
    );

    let (status, headers, bytes) =
        binary_request(app, Method::GET, &format!("/api/jobs/{id}/download")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers[header::CONTENT_TYPE], "application/epub+zip");
    let report = inspect_epub_bytes(&bytes);
    assert!(report.ok, "inspection errors: {:?}", report.errors);
    assert!(
        report
            .xhtml
            .iter()
            .flat_map(|xhtml| xhtml.srcs.iter())
            .all(|src| !src.starts_with("http") && !src.contains("missing.png")),
        "single chapter image references should be packaged and skip failed images: {:?}",
        report.xhtml
    );
    let package = report.package.expect("package should inspect");
    assert_eq!(package.content_chapters.len(), 1);
    assert!(
        package
            .manifest
            .iter()
            .any(|item| item.media_type.starts_with("image/")),
        "single include-images output did not package image resources"
    );
}

#[tokio::test]
async fn crawl_jobs_download_multi_chapter_image_epub_with_structured_warnings() {
    let app = fixture_app();
    let created = create_job(app.clone(), crawl_images_payload("epub")).await;
    let id = created["id"].as_str().expect("id should be present");

    let terminal = wait_for_terminal(app.clone(), id).await;
    assert_eq!(terminal["status"], "completed");
    assert_eq!(terminal["mode"], "crawl");
    assert_eq!(terminal["summary"]["options"]["includeImages"], true);
    assert_eq!(terminal["summary"]["options"]["outputTarget"], "epub");
    assert_eq!(
        terminal["summary"]["crawl"]["prefixUrl"],
        "https://example.test/images-crawl/"
    );
    assert_eq!(terminal["progress"]["pagesFetched"], 2);

    let warnings = terminal["warnings"].as_array().expect("warnings array");
    assert!(
        warnings.iter().any(|warning| {
            warning["code"] == "image_fetch_failed"
                && warning["affected"] == "https://example.test/images/missing.png"
        }),
        "missing-image warning was not surfaced: {warnings:#?}"
    );

    let (status, headers, bytes) =
        binary_request(app, Method::GET, &format!("/api/jobs/{id}/download")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers[header::CONTENT_TYPE], "application/epub+zip");
    let report = inspect_epub_bytes(&bytes);
    assert!(report.ok, "inspection errors: {:?}", report.errors);
    assert!(
        report
            .xhtml
            .iter()
            .flat_map(|xhtml| xhtml.srcs.iter())
            .all(|src| !src.contains("missing.png") && !src.starts_with("http")),
        "chapter image references should be packaged and skip failed images: {:?}",
        report.xhtml
    );

    let package = report.package.expect("package should inspect");
    let titles = package
        .nav_entries
        .iter()
        .map(|entry| entry.label.as_str())
        .collect::<Vec<_>>();
    assert_eq!(titles, vec!["Image Crawl Start", "Image Crawl Second"]);
    assert_eq!(package.content_chapters.len(), 2);
    assert!(
        package
            .manifest
            .iter()
            .filter(|item| item.media_type.starts_with("image/"))
            .count()
            >= 2
    );
}

#[tokio::test]
async fn reader_export_target_stays_epub_only_and_automation_shapes_are_rejected() {
    let app = fixture_app();
    let created = create_job(app.clone(), crawl_images_payload("weread")).await;
    let id = created["id"].as_str().expect("id should be present");
    let terminal = wait_for_terminal(app.clone(), id).await;
    assert_eq!(terminal["status"], "completed");
    assert_eq!(terminal["summary"]["options"]["outputTarget"], "weread");

    let (status, headers, bytes) = binary_request(
        app.clone(),
        Method::GET,
        &format!("/api/jobs/{id}/download"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers[header::CONTENT_TYPE], "application/epub+zip");
    assert!(
        headers[header::CONTENT_DISPOSITION]
            .to_str()
            .expect("content disposition should be ascii")
            .ends_with(".epub\"")
    );
    let report = inspect_epub_bytes(&bytes);
    assert!(report.ok, "inspection errors: {:?}", report.errors);

    let mut non_epub_target = single_payload(
        "https://example.test/single-page/index.html",
        "Non EPUB Target",
    );
    non_epub_target["options"]["outputTarget"] = json!("pdf");
    let mut direct_send = single_payload(
        "https://example.test/single-page/index.html",
        "Direct Send Target",
    );
    direct_send["options"]["directSend"] = json!(true);

    for payload in [non_epub_target, direct_send] {
        let (status, _, body) =
            json_request(app.clone(), Method::POST, "/api/jobs", Some(payload)).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body:#?}");
        assert_safe_error(&body);
        assert!(body.get("id").is_none());
    }
}

#[tokio::test]
async fn unsafe_url_matrix_rejects_before_artifacts_or_fails_with_safe_security_status() {
    let app = fixture_app();

    for (label, source_url, expected_field) in [
        ("malformed", "not a url", "sourceUrl"),
        ("unsupported scheme", "file:///etc/passwd", "sourceUrl"),
        ("loopback", "http://127.0.0.1/private", "sourceUrl"),
        (
            "metadata service",
            "http://169.254.169.254/latest/meta-data/",
            "sourceUrl",
        ),
    ] {
        let (status, _, body) = json_request(
            app.clone(),
            Method::POST,
            "/api/jobs",
            Some(single_payload(source_url, label)),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{label} should be rejected: {body:#?}"
        );
        assert_safe_error(&body);
        assert_eq!(body["error"]["code"], "validation_failed");
        assert!(
            body["error"]["fields"]
                .as_array()
                .is_some_and(|fields| fields.iter().any(|field| field == expected_field)),
            "{label} did not identify {expected_field}: {body:#?}"
        );
        assert!(body.get("id").is_none(), "{label} created a job");
    }

    let created = create_job(
        app.clone(),
        single_payload(
            "https://example.test/redirects/to-private",
            "Private Redirect",
        ),
    )
    .await;
    let id = created["id"].as_str().expect("id should be present");
    let terminal = wait_for_terminal(app.clone(), id).await;
    assert_eq!(terminal["status"], "failed");
    assert_eq!(terminal["errors"][0]["code"], "unsafe_url");
    assert_safe_error(&json!({"error": terminal["errors"][0]}));

    let (status, _, body) =
        json_request(app, Method::GET, &format!("/api/jobs/{id}/download"), None).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["error"]["code"], "job_failed");
    assert_safe_error(&body);
}

#[tokio::test]
async fn invalid_requests_unknown_jobs_and_failed_jobs_are_safe_json() {
    let app = fixture_app();

    for payload in [
        json!({"mode": "single"}),
        json!({
            "sourceUrl": "not a url",
            "mode": "single",
            "metadata": {"title": "Bad URL", "author": "A", "language": "en", "description": ""}
        }),
        json!({
            "sourceUrl": "https://example.test/single-page/index.html",
            "mode": "invalid",
            "metadata": {"title": "Bad Mode", "author": "A", "language": "en", "description": ""}
        }),
        json!({
            "sourceUrl": "https://example.test/crawl-graph/index.html",
            "mode": "crawl",
            "metadata": {"title": "Bad Crawl", "author": "A", "language": "en", "description": ""},
            "crawl": {"prefixUrl": "https://example.test/crawl-graph/", "maxDepth": 2, "maxPages": 0}
        }),
    ] {
        let (status, _, body) =
            json_request(app.clone(), Method::POST, "/api/jobs", Some(payload)).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body:#?}");
        assert_safe_error(&body);
        assert!(body.get("id").is_none());
    }

    let (status, _, body) =
        json_request(app.clone(), Method::GET, "/api/jobs/not-a-uuid", None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_safe_error(&body);
    assert_eq!(body["error"]["code"], "invalid_job_id");

    let missing = "00000000-0000-0000-0000-000000000000";
    let (status, _, body) = json_request(
        app.clone(),
        Method::GET,
        &format!("/api/jobs/{missing}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_safe_error(&body);
    assert_eq!(body["error"]["code"], "job_not_found");

    let failed = create_job(
        app.clone(),
        single_payload(
            "https://example.test/does-not-exist.html",
            "Missing Fixture",
        ),
    )
    .await;
    let failed = wait_for_terminal(app.clone(), failed["id"].as_str().expect("id")).await;
    assert_eq!(failed["status"], "failed");
    assert_safe_error(&json!({"error": failed["errors"][0]}));

    let oversized = create_job(
        app.clone(),
        single_payload(
            "https://example.test/oversized-slow/oversized.html",
            "Oversized Fixture",
        ),
    )
    .await;
    let oversized = wait_for_terminal(app.clone(), oversized["id"].as_str().expect("id")).await;
    assert_eq!(oversized["status"], "failed");
    assert_eq!(oversized["errors"][0]["code"], "response_too_large");
    assert!(
        oversized["errors"][0]["message"]
            .as_str()
            .is_some_and(|message| message.contains("byte limit"))
    );
    assert_safe_error(&json!({"error": oversized["errors"][0]}));

    let (status, _, body) = json_request(
        app,
        Method::GET,
        &format!(
            "/api/jobs/{}/download",
            failed["id"].as_str().expect("id should be present")
        ),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_safe_error(&body);
    assert_eq!(body["error"]["code"], "job_failed");
}

#[tokio::test]
async fn downloads_are_unavailable_before_completion() {
    let app = delayed_fixture_app(Duration::from_millis(250));
    let created = create_job(
        app.clone(),
        single_payload(
            "https://example.test/single-page/index.html",
            "Delayed Download Gate",
        ),
    )
    .await;
    let id = created["id"].as_str().expect("id should be present");

    let (status, _, body) = json_request(
        app.clone(),
        Method::GET,
        &format!("/api/jobs/{id}/download"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_safe_error(&body);
    assert_eq!(body["error"]["code"], "job_not_completed");

    let terminal = wait_for_terminal(app, id).await;
    assert_eq!(terminal["status"], "completed");
}

#[tokio::test]
async fn concurrent_jobs_keep_unique_ids_and_isolated_artifacts() {
    let app = fixture_app();
    let (first, second) = tokio::join!(
        create_job(
            app.clone(),
            single_payload(
                "https://example.test/single-page/index.html",
                "First API Book"
            )
        ),
        create_job(app.clone(), crawl_payload(2, 10))
    );
    let first_id = first["id"].as_str().expect("first id");
    let second_id = second["id"].as_str().expect("second id");
    assert_ne!(first_id, second_id);

    let (first_terminal, second_terminal) = tokio::join!(
        wait_for_terminal(app.clone(), first_id),
        wait_for_terminal(app.clone(), second_id)
    );
    assert_eq!(first_terminal["status"], "completed");
    assert_eq!(second_terminal["status"], "completed");
    assert_eq!(first_terminal["mode"], "single");
    assert_eq!(second_terminal["mode"], "crawl");
    assert_ne!(
        first_terminal["summary"]["metadata"]["title"],
        second_terminal["summary"]["metadata"]["title"]
    );

    let (first_status, first_headers, first_bytes) = binary_request(
        app.clone(),
        Method::GET,
        &format!("/api/jobs/{first_id}/download"),
    )
    .await;
    let (second_status, second_headers, second_bytes) =
        binary_request(app, Method::GET, &format!("/api/jobs/{second_id}/download")).await;
    assert_eq!(first_status, StatusCode::OK);
    assert_eq!(second_status, StatusCode::OK);
    assert_ne!(
        first_headers[header::CONTENT_DISPOSITION],
        second_headers[header::CONTENT_DISPOSITION]
    );
    assert_ne!(first_bytes, second_bytes);
}
