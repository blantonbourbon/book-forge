use std::{
    future::Future,
    net::SocketAddr,
    path::{Component, Path, PathBuf},
    pin::Pin,
    sync::Arc,
    time::Duration,
};

use thiserror::Error;
use tokio::time::sleep;
use url::Url;

use crate::security;

pub type FetchFuture = Pin<Box<dyn Future<Output = Result<FetchedResponse, FetchError>> + Send>>;

const MAX_REDIRECTS: usize = 5;

#[derive(Clone, Debug)]
pub struct FetchedResponse {
    pub final_url: String,
    pub media_type: String,
    pub bytes: Vec<u8>,
}

impl FetchedResponse {
    pub fn text(self) -> Result<String, FetchError> {
        String::from_utf8(self.bytes).map_err(|_| {
            FetchError::new("invalid_text", "Fetched content was not valid UTF-8 text.")
        })
    }
}

#[derive(Clone, Debug, Error)]
#[error("{message}")]
pub struct FetchError {
    pub code: String,
    pub message: String,
}

impl FetchError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

pub trait Fetcher: Send + Sync + 'static {
    fn fetch(&self, url: Url, timeout: Duration, max_bytes: usize) -> FetchFuture;
}

#[derive(Clone)]
pub struct SharedFetcher(pub Arc<dyn Fetcher>);

impl SharedFetcher {
    pub fn fixture_or_http() -> Self {
        Self(Arc::new(FixtureOrHttpFetcher::new(Duration::ZERO)))
    }

    pub fn fixture_or_http_with_delay(delay: Duration) -> Self {
        Self(Arc::new(FixtureOrHttpFetcher::new(delay)))
    }

    pub fn fixture_or_http_with_resolved_host(domain: &str, addrs: &[SocketAddr]) -> Self {
        Self(Arc::new(FixtureOrHttpFetcher::new_with_overrides(
            Duration::ZERO,
            &[(domain, addrs)],
        )))
    }
}

impl Fetcher for SharedFetcher {
    fn fetch(&self, url: Url, timeout: Duration, max_bytes: usize) -> FetchFuture {
        self.0.fetch(url, timeout, max_bytes)
    }
}

#[derive(Clone)]
struct FixtureOrHttpFetcher {
    client: reqwest::Client,
    fixture_root: PathBuf,
    fixture_delay: Duration,
    http_override_hosts: Vec<String>,
}

impl FixtureOrHttpFetcher {
    fn new(fixture_delay: Duration) -> Self {
        Self::new_with_overrides(fixture_delay, &[])
    }

    fn new_with_overrides(fixture_delay: Duration, overrides: &[(&str, &[SocketAddr])]) -> Self {
        let fixture_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures");
        let fixture_root = fixture_root.canonicalize().unwrap_or(fixture_root);
        let mut builder = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy();
        for (domain, addrs) in overrides {
            builder = builder.resolve_to_addrs(domain, addrs);
        }
        let http_override_hosts = overrides
            .iter()
            .map(|(domain, _)| domain.to_ascii_lowercase())
            .collect();

        Self {
            client: builder.build().expect("reqwest client should build"),
            fixture_root,
            fixture_delay,
            http_override_hosts,
        }
    }
}

impl Fetcher for FixtureOrHttpFetcher {
    fn fetch(&self, url: Url, timeout: Duration, max_bytes: usize) -> FetchFuture {
        let client = self.client.clone();
        let fixture_root = self.fixture_root.clone();
        let fixture_delay = self.fixture_delay;
        let http_override_hosts = self.http_override_hosts.clone();

        Box::pin(async move {
            let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
            if host == "example.test" && !http_override_hosts.contains(&host) {
                tokio::time::timeout(
                    timeout,
                    fetch_fixture(&fixture_root, url, fixture_delay, max_bytes),
                )
                .await
                .map_err(|_| {
                    FetchError::new("fetch_timeout", "Fetching source content timed out.")
                })?
            } else {
                fetch_http(client, url, timeout, max_bytes).await
            }
        })
    }
}

async fn fetch_fixture(
    fixture_root: &Path,
    mut url: Url,
    fixture_delay: Duration,
    max_bytes: usize,
) -> Result<FetchedResponse, FetchError> {
    for redirect_count in 0..=MAX_REDIRECTS {
        security::validate_network_url(&url)
            .await
            .map_err(security_fetch_error)?;

        let total_delay = fixture_delay.saturating_add(fixture_route_delay(&url));
        if !total_delay.is_zero() {
            sleep(total_delay).await;
        }

        if let Some(location) = fixture_redirect_location(&url) {
            if redirect_count == MAX_REDIRECTS {
                return Err(FetchError::new(
                    "redirect_limit_exceeded",
                    "Redirect handling exceeded the configured limit.",
                ));
            }
            url = url.join(location).map_err(|_| {
                FetchError::new("invalid_redirect", "Redirect target was not a valid URL.")
            })?;
            continue;
        }

        let relative_path = fixture_relative_path(&url).ok_or_else(|| {
            FetchError::new(
                "fixture_not_found",
                "The deterministic fixture content was not available.",
            )
        })?;
        let path = fixture_root.join(relative_path);
        ensure_within_root(fixture_root, &path)?;

        let bytes = tokio::fs::read(&path).await.map_err(|_| {
            FetchError::new(
                "fetch_failed",
                "The deterministic fixture content was not available.",
            )
        })?;
        let declared_bytes = fixture_declared_bytes(&url).unwrap_or(bytes.len());
        if declared_bytes > max_bytes || bytes.len() > max_bytes {
            return Err(FetchError::new(
                "response_too_large",
                "Fetched content exceeded the configured byte limit.",
            ));
        }

        return Ok(FetchedResponse {
            final_url: url.to_string(),
            media_type: media_type_for_path(&path),
            bytes,
        });
    }

    Err(FetchError::new(
        "redirect_limit_exceeded",
        "Redirect handling exceeded the configured limit.",
    ))
}

async fn fetch_http(
    client: reqwest::Client,
    mut url: Url,
    timeout: Duration,
    max_bytes: usize,
) -> Result<FetchedResponse, FetchError> {
    let future =
        async {
            for redirect_count in 0..=MAX_REDIRECTS {
                let resolved_target = security::resolve_vetted_addrs(&url)
                    .await
                    .map_err(security_fetch_error)?;
                canonicalize_outbound_host(&mut url)?;
                let request_client = client_for_resolved_target(&client, resolved_target.as_ref())?;

                let mut response = request_client
                    .get(url.clone())
                    .header(reqwest::header::USER_AGENT, "BookForge/0.1")
                    .send()
                    .await
                    .map_err(|_| {
                        FetchError::new("fetch_failed", "Source content could not be fetched.")
                    })?;

                let status = response.status();
                if status.is_redirection() {
                    if redirect_count == MAX_REDIRECTS {
                        return Err(FetchError::new(
                            "redirect_limit_exceeded",
                            "Redirect handling exceeded the configured limit.",
                        ));
                    }
                    let location = response
                        .headers()
                        .get(reqwest::header::LOCATION)
                        .and_then(|value| value.to_str().ok())
                        .ok_or_else(|| {
                            FetchError::new(
                                "invalid_redirect",
                                "Redirect response did not include a valid target.",
                            )
                        })?;
                    url = url.join(location).map_err(|_| {
                        FetchError::new("invalid_redirect", "Redirect target was not a valid URL.")
                    })?;
                    continue;
                }

                if !status.is_success() {
                    return Err(FetchError::new(
                        "fetch_failed",
                        format!("Source returned HTTP status {}.", status.as_u16()),
                    ));
                }

                if response
                    .content_length()
                    .is_some_and(|length| length > max_bytes as u64)
                {
                    return Err(response_too_large_error());
                }

                let final_url = response.url().to_string();
                let media_type = response
                    .headers()
                    .get(reqwest::header::CONTENT_TYPE)
                    .and_then(|value| value.to_str().ok())
                    .and_then(|value| value.split(';').next())
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .unwrap_or("application/octet-stream")
                    .to_string();
                let mut bytes = Vec::new();
                while let Some(chunk) = response.chunk().await.map_err(|_| {
                    FetchError::new("fetch_failed", "Source body could not be read.")
                })? {
                    if bytes.len().saturating_add(chunk.len()) > max_bytes {
                        return Err(response_too_large_error());
                    }
                    bytes.extend_from_slice(&chunk);
                }

                return Ok(FetchedResponse {
                    final_url,
                    media_type,
                    bytes,
                });
            }

            Err(FetchError::new(
                "redirect_limit_exceeded",
                "Redirect handling exceeded the configured limit.",
            ))
        };

    tokio::time::timeout(timeout, future)
        .await
        .map_err(|_| FetchError::new("fetch_timeout", "Fetching source content timed out."))?
}

fn canonicalize_outbound_host(url: &mut Url) -> Result<(), FetchError> {
    let Some(canonical_host) = security::canonical_domain_for_outbound_request(url) else {
        return Ok(());
    };
    if url.host_str() == Some(canonical_host.as_str()) {
        return Ok(());
    }
    url.set_host(Some(&canonical_host)).map_err(|_| {
        FetchError::new(
            "unsafe_url",
            "Source URL host was not accepted for outbound fetching.",
        )
    })
}

fn client_for_resolved_target(
    base_client: &reqwest::Client,
    resolved_target: Option<&security::VettedResolvedAddrs>,
) -> Result<reqwest::Client, FetchError> {
    let Some(resolved_target) = resolved_target else {
        return Ok(base_client.clone());
    };

    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .resolve_to_addrs(&resolved_target.domain, &resolved_target.addresses)
        .build()
        .map_err(|_| FetchError::new("fetch_failed", "HTTP client could not be prepared."))
}

fn response_too_large_error() -> FetchError {
    FetchError::new(
        "response_too_large",
        "Fetched content exceeded the configured byte limit.",
    )
}

fn security_fetch_error(error: security::SecurityError) -> FetchError {
    FetchError::new(error.code, error.message)
}

fn fixture_relative_path(url: &Url) -> Option<PathBuf> {
    let segments = url.path_segments()?.collect::<Vec<_>>();
    if segments.is_empty()
        || segments
            .iter()
            .any(|segment| segment.is_empty() || unsafe_path_segment(segment))
    {
        return None;
    }

    let mut relative = PathBuf::new();
    if segments.first() == Some(&"images") {
        relative.push("images");
        for segment in segments.iter().skip(1) {
            relative.push(segment);
        }
    } else {
        relative.push("html");
        for segment in segments {
            relative.push(segment);
        }
    }
    Some(relative)
}

fn unsafe_path_segment(segment: &str) -> bool {
    let lower = segment.to_lowercase();
    segment == "."
        || segment == ".."
        || lower.contains("%2e")
        || lower.contains("%2f")
        || lower.contains('\\')
}

fn ensure_within_root(root: &Path, path: &Path) -> Result<(), FetchError> {
    let mut depth = 0usize;
    for component in path.components() {
        match component {
            Component::ParentDir => {
                return Err(FetchError::new(
                    "fetch_failed",
                    "Fixture path was not accepted.",
                ));
            }
            Component::Normal(_) => depth += 1,
            _ => {}
        }
    }

    if depth == 0 || !path.starts_with(root) {
        return Err(FetchError::new(
            "fetch_failed",
            "Fixture path was not accepted.",
        ));
    }
    Ok(())
}

fn media_type_for_path(path: &Path) -> String {
    match path.extension().and_then(|extension| extension.to_str()) {
        Some("html" | "htm") => "text/html; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        _ => "application/octet-stream",
    }
    .to_string()
}

fn fixture_redirect_location(url: &Url) -> Option<&'static str> {
    match url.path() {
        "/redirects/to-safe" => Some("/single-page/index.html"),
        "/redirects/to-private" => Some("http://127.0.0.1:3100/private-target"),
        "/redirects/loop-a" => Some("/redirects/loop-b"),
        "/redirects/loop-b" => Some("/redirects/loop-a"),
        _ => None,
    }
}

fn fixture_route_delay(url: &Url) -> Duration {
    if url.path() == "/oversized-slow/slow.html" {
        Duration::from_millis(3_500)
    } else {
        Duration::ZERO
    }
}

fn fixture_declared_bytes(url: &Url) -> Option<usize> {
    (url.path() == "/oversized-slow/oversized.html").then_some(10_485_761)
}
