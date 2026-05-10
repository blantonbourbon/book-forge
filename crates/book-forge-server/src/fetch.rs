use std::{
    future::Future,
    path::{Component, Path, PathBuf},
    pin::Pin,
    sync::Arc,
    time::Duration,
};

use thiserror::Error;
use tokio::time::sleep;
use url::Url;

pub type FetchFuture = Pin<Box<dyn Future<Output = Result<FetchedResponse, FetchError>> + Send>>;

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
}

impl FixtureOrHttpFetcher {
    fn new(fixture_delay: Duration) -> Self {
        let fixture_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures");
        let fixture_root = fixture_root.canonicalize().unwrap_or(fixture_root);
        Self {
            client: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::limited(5))
                .build()
                .expect("reqwest client should build"),
            fixture_root,
            fixture_delay,
        }
    }
}

impl Fetcher for FixtureOrHttpFetcher {
    fn fetch(&self, url: Url, timeout: Duration, max_bytes: usize) -> FetchFuture {
        let client = self.client.clone();
        let fixture_root = self.fixture_root.clone();
        let fixture_delay = self.fixture_delay;

        Box::pin(async move {
            if url.host_str() == Some("example.test") {
                fetch_fixture(&fixture_root, url, fixture_delay, max_bytes).await
            } else {
                fetch_http(client, url, timeout, max_bytes).await
            }
        })
    }
}

async fn fetch_fixture(
    fixture_root: &Path,
    url: Url,
    fixture_delay: Duration,
    max_bytes: usize,
) -> Result<FetchedResponse, FetchError> {
    let total_delay = fixture_delay.saturating_add(fixture_route_delay(&url));
    if !total_delay.is_zero() {
        sleep(total_delay).await;
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

    Ok(FetchedResponse {
        final_url: url.to_string(),
        media_type: media_type_for_path(&path),
        bytes,
    })
}

async fn fetch_http(
    client: reqwest::Client,
    url: Url,
    timeout: Duration,
    max_bytes: usize,
) -> Result<FetchedResponse, FetchError> {
    let future = async {
        let response = client
            .get(url.clone())
            .header(reqwest::header::USER_AGENT, "BookForge/0.1")
            .send()
            .await
            .map_err(|_| FetchError::new("fetch_failed", "Source content could not be fetched."))?;

        let status = response.status();
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
            return Err(FetchError::new(
                "response_too_large",
                "Fetched content exceeded the configured byte limit.",
            ));
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
        let bytes = response
            .bytes()
            .await
            .map_err(|_| FetchError::new("fetch_failed", "Source body could not be read."))?
            .to_vec();
        if bytes.len() > max_bytes {
            return Err(FetchError::new(
                "response_too_large",
                "Fetched content exceeded the configured byte limit.",
            ));
        }

        Ok(FetchedResponse {
            final_url,
            media_type,
            bytes,
        })
    };

    tokio::time::timeout(timeout, future)
        .await
        .map_err(|_| FetchError::new("fetch_timeout", "Fetching source content timed out."))?
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
