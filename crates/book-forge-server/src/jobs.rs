use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

use book_forge_converter::{
    BookMetadata, ConversionError, ConversionOptions, ConversionResult, ConversionWarning,
    CrawlInput, CrawlOptions, CrawlPage, CrawlResource, SinglePageInput, convert_crawl,
    convert_single_page,
};
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use url::Url;
use uuid::Uuid;

use crate::{
    errors::{ApiError, ErrorBody, sanitize_message},
    fetch::{FetchError, FetchedResponse, Fetcher, SharedFetcher},
    security,
};

const DEFAULT_FETCH_TIMEOUT_MILLIS: u64 = 30_000;
const DEFAULT_MAX_TOTAL_BYTES: usize = 10 * 1024 * 1024;
const MAX_METADATA_CHARS: usize = 512;
const MAX_CRAWL_DEPTH: usize = 10;
const MAX_CRAWL_PAGES: usize = 100;
const MAX_TOTAL_BYTES: usize = 20 * 1024 * 1024;
const MAX_DURATION_MILLIS: u64 = 120_000;

#[derive(Clone)]
pub struct AppState {
    pub jobs: JobManager,
    pub fetcher: SharedFetcher,
    pub static_root: Option<Arc<PathBuf>>,
}

impl AppState {
    pub fn new(fetcher: SharedFetcher) -> Self {
        Self {
            jobs: JobManager::default(),
            fetcher,
            static_root: None,
        }
    }

    pub fn with_static_root(mut self, static_root: PathBuf) -> Self {
        self.static_root = static_root.canonicalize().ok().map(Arc::new);
        self
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateJobRequest {
    pub source_url: Option<String>,
    pub mode: Option<String>,
    #[serde(default)]
    pub metadata: Option<ApiMetadata>,
    #[serde(default)]
    pub options: ApiOptions,
    #[serde(default)]
    pub crawl: Option<ApiCrawlOptions>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum JobMode {
    Single,
    Crawl,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ApiMetadata {
    pub title: Option<String>,
    pub author: Option<String>,
    pub language: Option<String>,
    pub description: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ApiOutputTarget {
    #[default]
    Epub,
    Weread,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ApiOptions {
    #[serde(default)]
    pub include_images: bool,
    #[serde(default)]
    pub output_target: ApiOutputTarget,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ApiCrawlOptions {
    pub prefix_url: Option<String>,
    pub max_depth: Option<usize>,
    pub max_pages: Option<usize>,
    pub max_total_bytes: Option<usize>,
    pub max_duration_millis: Option<u64>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobResponse {
    pub id: String,
    pub status: JobStatus,
    pub mode: JobMode,
    pub summary: JobSummary,
    pub progress: JobProgress,
    pub warnings: Vec<ConversionWarning>,
    pub errors: Vec<ErrorBody>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub download_url: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum JobStatus {
    Queued,
    Running,
    Completed,
    Failed,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobSummary {
    pub source_url: String,
    pub mode: JobMode,
    pub metadata: BookMetadata,
    pub options: ApiOptions,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub crawl: Option<CrawlSummary>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CrawlSummary {
    pub prefix_url: String,
    pub max_depth: usize,
    pub max_pages: usize,
    pub max_total_bytes: usize,
    pub max_duration_millis: u64,
}

impl From<CrawlSummary> for CrawlOptions {
    fn from(value: CrawlSummary) -> Self {
        Self {
            prefix_url: value.prefix_url,
            max_depth: value.max_depth,
            max_pages: value.max_pages,
            max_total_bytes: value.max_total_bytes,
            max_duration_millis: value.max_duration_millis,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobProgress {
    pub percent: u8,
    pub pages_discovered: usize,
    pub pages_fetched: usize,
    pub pages_skipped: usize,
    pub current_depth: usize,
    pub bytes_fetched: usize,
    pub max_pages: usize,
    pub max_depth: usize,
    pub max_total_bytes: usize,
}

impl JobProgress {
    fn queued(summary: &JobSummary) -> Self {
        let crawl = summary.crawl.as_ref();
        Self {
            percent: 0,
            pages_discovered: 0,
            pages_fetched: 0,
            pages_skipped: 0,
            current_depth: 0,
            bytes_fetched: 0,
            max_pages: crawl.map_or(1, |crawl| crawl.max_pages),
            max_depth: crawl.map_or(0, |crawl| crawl.max_depth),
            max_total_bytes: crawl.map_or(DEFAULT_MAX_TOTAL_BYTES, |crawl| crawl.max_total_bytes),
        }
    }

    fn running(summary: &JobSummary) -> Self {
        let mut progress = Self::queued(summary);
        progress.percent = 5;
        progress
    }

    fn completed(mut self) -> Self {
        self.percent = 100;
        self
    }

    fn failed(mut self) -> Self {
        self.percent = self.percent.min(99);
        self
    }
}

#[derive(Clone, Debug)]
pub struct Artifact {
    pub filename: String,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug)]
struct JobRecord {
    id: Uuid,
    status: JobStatus,
    summary: JobSummary,
    progress: JobProgress,
    warnings: Vec<ConversionWarning>,
    errors: Vec<ErrorBody>,
    artifact: Option<Artifact>,
}

impl JobRecord {
    fn new(id: Uuid, summary: JobSummary) -> Self {
        Self {
            id,
            status: JobStatus::Queued,
            progress: JobProgress::queued(&summary),
            summary,
            warnings: Vec::new(),
            errors: Vec::new(),
            artifact: None,
        }
    }

    fn response(&self) -> JobResponse {
        JobResponse {
            id: self.id.to_string(),
            status: self.status,
            mode: self.summary.mode,
            summary: self.summary.clone(),
            progress: self.progress.clone(),
            warnings: self.warnings.clone(),
            errors: self.errors.clone(),
            download_url: (self.status == JobStatus::Completed)
                .then(|| format!("/api/jobs/{}/download", self.id)),
        }
    }
}

#[derive(Clone, Default)]
pub struct JobManager {
    inner: Arc<RwLock<HashMap<Uuid, JobRecord>>>,
}

impl JobManager {
    pub async fn create_job(&self, fetcher: SharedFetcher, summary: JobSummary) -> JobResponse {
        let id = Uuid::new_v4();
        let record = JobRecord::new(id, summary.clone());
        self.inner.write().await.insert(id, record);

        let jobs = self.clone();
        tokio::spawn(async move {
            jobs.mark_running(id).await;
            match execute_job(id, jobs.clone(), fetcher, summary).await {
                Ok((result, progress)) => jobs.mark_completed(id, result, progress).await,
                Err((error, progress)) => jobs.mark_failed(id, error, progress).await,
            }
        });

        self.get_response(id)
            .await
            .expect("job should exist immediately after insertion")
    }

    pub async fn get_response(&self, id: Uuid) -> Option<JobResponse> {
        self.inner.read().await.get(&id).map(JobRecord::response)
    }

    pub async fn artifact(&self, id: Uuid) -> Option<(JobStatus, Option<Artifact>)> {
        self.inner
            .read()
            .await
            .get(&id)
            .map(|job| (job.status, job.artifact.clone()))
    }

    async fn mark_running(&self, id: Uuid) {
        if let Some(job) = self.inner.write().await.get_mut(&id) {
            job.status = JobStatus::Running;
            job.progress = JobProgress::running(&job.summary);
        }
    }

    async fn update_progress(&self, id: Uuid, progress: JobProgress) {
        if let Some(job) = self.inner.write().await.get_mut(&id)
            && job.status == JobStatus::Running
        {
            job.progress = progress;
        }
    }

    async fn mark_completed(&self, id: Uuid, result: ConversionResult, progress: JobProgress) {
        if let Some(job) = self.inner.write().await.get_mut(&id) {
            job.status = JobStatus::Completed;
            job.progress = progress.completed();
            job.warnings = result.warnings;
            job.errors.clear();
            job.artifact = Some(Artifact {
                filename: result.download_filename,
                bytes: result.epub_bytes,
            });
        }
    }

    async fn mark_failed(&self, id: Uuid, error: ErrorBody, progress: JobProgress) {
        if let Some(job) = self.inner.write().await.get_mut(&id) {
            job.status = JobStatus::Failed;
            job.progress = progress.failed();
            job.errors = vec![error];
            job.artifact = None;
        }
    }
}

pub fn validate_create_request(request: CreateJobRequest) -> Result<JobSummary, ApiError> {
    let mut fields = Vec::new();

    let Some(source_url) = request.source_url.map(trim_field) else {
        fields.push("sourceUrl".to_string());
        return Err(ApiError::validation("A source URL is required.", fields));
    };
    if source_url.is_empty() {
        fields.push("sourceUrl".to_string());
    }

    let parsed_source = Url::parse(&source_url).ok();
    match parsed_source.as_ref().map(Url::scheme) {
        Some("http" | "https") => {}
        _ => fields.push("sourceUrl".to_string()),
    }

    let Some(raw_mode) = request.mode.map(trim_field) else {
        fields.push("mode".to_string());
        return Err(ApiError::validation(
            "A conversion mode is required.",
            fields,
        ));
    };
    let mode = match raw_mode.as_str() {
        "single" => JobMode::Single,
        "crawl" => JobMode::Crawl,
        _ => {
            fields.push("mode".to_string());
            JobMode::Single
        }
    };

    let metadata = metadata_from_request(request.metadata.unwrap_or_default(), &mut fields);
    let crawl = if mode == JobMode::Crawl {
        let source = parsed_source.as_ref().ok_or_else(|| {
            ApiError::validation("Source URL must be absolute HTTP or HTTPS.", fields.clone())
        })?;
        Some(crawl_from_request(
            request.crawl.unwrap_or_default(),
            source,
            &mut fields,
        ))
    } else {
        None
    };

    if !fields.is_empty() {
        return Err(ApiError::validation(
            "One or more job request fields were invalid.",
            fields,
        ));
    }

    Ok(JobSummary {
        source_url,
        mode,
        metadata,
        options: request.options,
        crawl,
    })
}

pub async fn enforce_create_request_security(summary: &JobSummary) -> Result<(), ApiError> {
    let source_url = Url::parse(&summary.source_url).map_err(|_| {
        ApiError::validation(
            "Source URL must be absolute HTTP or HTTPS.",
            vec!["sourceUrl".to_string()],
        )
    })?;
    security::validate_network_url(&source_url)
        .await
        .map_err(|error| ApiError::validation(error.message, vec!["sourceUrl".to_string()]))?;

    if let Some(crawl) = &summary.crawl {
        let prefix_url = Url::parse(&crawl.prefix_url).map_err(|_| {
            ApiError::validation(
                "Crawl prefix URL must be absolute HTTP or HTTPS.",
                vec!["crawl.prefixUrl".to_string()],
            )
        })?;
        security::validate_network_url(&prefix_url)
            .await
            .map_err(|error| {
                ApiError::validation(error.message, vec!["crawl.prefixUrl".to_string()])
            })?;
    }

    Ok(())
}

fn metadata_from_request(metadata: ApiMetadata, fields: &mut Vec<String>) -> BookMetadata {
    let title = metadata
        .title
        .map(trim_field)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "Untitled Book".to_string());
    let author = metadata
        .author
        .map(trim_field)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "Unknown Author".to_string());
    let language = metadata
        .language
        .map(trim_field)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "en".to_string());
    let description = metadata.description.map(trim_field).unwrap_or_default();

    for (field, value) in [
        ("metadata.title", &title),
        ("metadata.author", &author),
        ("metadata.language", &language),
        ("metadata.description", &description),
    ] {
        if value.chars().count() > MAX_METADATA_CHARS || value.chars().any(is_forbidden_control) {
            fields.push(field.to_string());
        }
    }

    if !valid_language_tag(&language) {
        fields.push("metadata.language".to_string());
    }

    BookMetadata {
        title,
        author,
        language,
        description,
    }
}

fn crawl_from_request(
    crawl: ApiCrawlOptions,
    source_url: &Url,
    fields: &mut Vec<String>,
) -> CrawlSummary {
    let prefix_url = crawl
        .prefix_url
        .map(trim_field)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| default_prefix_url(source_url));
    let parsed_prefix = Url::parse(&prefix_url).ok();
    match parsed_prefix.as_ref().map(Url::scheme) {
        Some("http" | "https") => {}
        _ => fields.push("crawl.prefixUrl".to_string()),
    }

    let max_depth = crawl.max_depth.unwrap_or(3);
    let max_pages = crawl.max_pages.unwrap_or(50);
    let max_total_bytes = crawl.max_total_bytes.unwrap_or(DEFAULT_MAX_TOTAL_BYTES);
    let max_duration_millis = crawl
        .max_duration_millis
        .unwrap_or(DEFAULT_FETCH_TIMEOUT_MILLIS);

    if max_depth > MAX_CRAWL_DEPTH {
        fields.push("crawl.maxDepth".to_string());
    }
    if max_pages == 0 || max_pages > MAX_CRAWL_PAGES {
        fields.push("crawl.maxPages".to_string());
    }
    if max_total_bytes == 0 || max_total_bytes > MAX_TOTAL_BYTES {
        fields.push("crawl.maxTotalBytes".to_string());
    }
    if max_duration_millis == 0 || max_duration_millis > MAX_DURATION_MILLIS {
        fields.push("crawl.maxDurationMillis".to_string());
    }

    CrawlSummary {
        prefix_url,
        max_depth,
        max_pages,
        max_total_bytes,
        max_duration_millis,
    }
}

async fn execute_job(
    id: Uuid,
    jobs: JobManager,
    fetcher: SharedFetcher,
    summary: JobSummary,
) -> Result<(ConversionResult, JobProgress), (ErrorBody, JobProgress)> {
    match summary.mode {
        JobMode::Single => execute_single(fetcher, summary).await,
        JobMode::Crawl => execute_crawl(id, jobs, fetcher, summary).await,
    }
}

async fn execute_single(
    fetcher: SharedFetcher,
    summary: JobSummary,
) -> Result<(ConversionResult, JobProgress), (ErrorBody, JobProgress)> {
    let mut progress = JobProgress::running(&summary);
    let source_url = Url::parse(&summary.source_url).expect("validated source URL should parse");
    let fetched = fetch_html(
        &fetcher,
        source_url,
        DEFAULT_FETCH_TIMEOUT_MILLIS,
        DEFAULT_MAX_TOTAL_BYTES,
    )
    .await
    .map_err(|error| (fetch_error_body(error), progress.clone()))?;

    progress.pages_discovered = 1;
    progress.pages_fetched = 1;
    progress.bytes_fetched = fetched.bytes.len();
    progress.percent = 70;

    let html = fetched
        .clone()
        .text()
        .map_err(|error| (fetch_error_body(error), progress.clone()))?;
    let result = convert_single_page(SinglePageInput {
        source_url: fetched.final_url,
        html,
        resources: Vec::new(),
        metadata: summary.metadata,
        options: ConversionOptions {
            include_images: summary.options.include_images,
        },
    })
    .map_err(|error| (conversion_error_body(error), progress.clone()))?;

    Ok((result, progress))
}

async fn execute_crawl(
    id: Uuid,
    jobs: JobManager,
    fetcher: SharedFetcher,
    summary: JobSummary,
) -> Result<(ConversionResult, JobProgress), (ErrorBody, JobProgress)> {
    let crawl = summary
        .crawl
        .clone()
        .expect("crawl summary should exist for crawl jobs");
    let crawl_options: CrawlOptions = crawl.clone().into();
    let source_url = Url::parse(&summary.source_url).expect("validated source URL should parse");
    let prefix_url = Url::parse(&crawl.prefix_url).expect("validated prefix URL should parse");
    let mut progress = JobProgress::running(&summary);
    let mut pages = Vec::new();
    let mut resources = Vec::new();
    let started = Instant::now();
    let mut queue = VecDeque::from([(source_url.clone(), 0usize)]);
    let mut seen_pages = HashSet::from([normalize_page_key(&source_url)]);
    let mut seen_resources = HashSet::new();

    progress.pages_discovered = 1;
    progress.percent = 10;
    jobs.update_progress(id, progress.clone()).await;

    while let Some((page_url, depth)) = queue.pop_front() {
        if started.elapsed() > Duration::from_millis(crawl.max_duration_millis) {
            break;
        }

        progress.current_depth = progress.current_depth.max(depth);
        let fetched = match fetch_html(
            &fetcher,
            page_url.clone(),
            crawl.max_duration_millis,
            crawl.max_total_bytes,
        )
        .await
        {
            Ok(fetched) => fetched,
            Err(error) if depth == 0 => {
                return Err((fetch_error_body(error), progress));
            }
            Err(error) => {
                progress.pages_skipped += 1;
                pages.push(CrawlPage {
                    url: page_url.to_string(),
                    html: None,
                    failure: Some(error.message),
                });
                continue;
            }
        };

        let page_bytes = fetched.bytes.len();
        if progress.bytes_fetched.saturating_add(page_bytes) > crawl.max_total_bytes {
            pages.push(CrawlPage {
                url: page_url.to_string(),
                html: None,
                failure: Some("crawl byte limit reached".to_string()),
            });
            progress.pages_skipped += 1;
            continue;
        }

        let final_url = fetched.final_url.clone();
        let html = fetched
            .text()
            .map_err(|error| (fetch_error_body(error), progress.clone()))?;
        progress.bytes_fetched += page_bytes;
        progress.pages_fetched += 1;
        progress.percent = 30
            + ((progress.pages_fetched.min(crawl.max_pages) * 50) / crawl.max_pages.max(1)) as u8;
        pages.push(CrawlPage {
            url: final_url.clone(),
            html: Some(html.clone()),
            failure: None,
        });

        if summary.options.include_images {
            for image_url in extract_image_urls(&html, &page_url) {
                if !matches!(image_url.scheme(), "http" | "https") {
                    continue;
                }
                let resource_key = normalize_resource_key(&image_url);
                if !seen_resources.insert(resource_key) {
                    continue;
                }
                match fetcher
                    .fetch(
                        image_url.clone(),
                        Duration::from_millis(crawl.max_duration_millis),
                        crawl.max_total_bytes,
                    )
                    .await
                {
                    Ok(resource) => resources.push(CrawlResource {
                        url: image_url.to_string(),
                        media_type: resource.media_type,
                        bytes: resource.bytes,
                        failure: None,
                    }),
                    Err(error) => resources.push(CrawlResource {
                        url: image_url.to_string(),
                        media_type: "application/octet-stream".to_string(),
                        bytes: Vec::new(),
                        failure: Some(error.message),
                    }),
                }
            }
        }

        for link_url in extract_link_urls(&html, &page_url) {
            if !is_in_crawl_scope(&link_url, &source_url, &prefix_url) {
                continue;
            }
            let key = normalize_page_key(&link_url);
            if seen_pages.contains(&key) {
                continue;
            }
            if depth.saturating_add(1) > crawl.max_depth || seen_pages.len() >= crawl.max_pages {
                continue;
            }
            seen_pages.insert(key);
            progress.pages_discovered = seen_pages.len();
            queue.push_back((without_fragment(&link_url), depth + 1));
        }

        jobs.update_progress(id, progress.clone()).await;
    }

    let mut result = convert_crawl(CrawlInput {
        start_url: summary.source_url,
        pages,
        resources,
        metadata: summary.metadata,
        options: ConversionOptions {
            include_images: summary.options.include_images,
        },
        crawl: crawl_options,
    })
    .map_err(|error| (conversion_error_body(error), progress.clone()))?;

    result.warnings = result
        .warnings
        .into_iter()
        .map(|warning| ConversionWarning {
            code: sanitize_message(warning.code),
            message: sanitize_message(warning.message),
            affected: warning.affected.map(sanitize_message),
        })
        .collect();
    Ok((result, progress))
}

async fn fetch_html(
    fetcher: &impl Fetcher,
    url: Url,
    timeout_millis: u64,
    max_bytes: usize,
) -> Result<FetchedResponse, FetchError> {
    let fetched = fetcher
        .fetch(url, Duration::from_millis(timeout_millis), max_bytes)
        .await?;
    if !is_html_like(&fetched.media_type) {
        return Err(FetchError::new(
            "unsupported_media_type",
            "Fetched content was not an HTML document.",
        ));
    }
    Ok(fetched)
}

fn conversion_error_body(error: ConversionError) -> ErrorBody {
    ErrorBody {
        code: error.code().to_string(),
        message: sanitize_message(error.safe_message()),
        fields: Vec::new(),
    }
}

fn fetch_error_body(error: FetchError) -> ErrorBody {
    ErrorBody {
        code: sanitize_message(error.code),
        message: sanitize_message(error.message),
        fields: Vec::new(),
    }
}

fn extract_link_urls(html: &str, base: &Url) -> Vec<Url> {
    extract_urls(html, base, "a[href]", "href")
}

fn extract_image_urls(html: &str, base: &Url) -> Vec<Url> {
    extract_urls(html, base, "img[src]", "src")
}

fn extract_urls(html: &str, base: &Url, selector: &str, attribute: &str) -> Vec<Url> {
    let document = Html::parse_document(html);
    let selector = Selector::parse(selector).expect("static selector should parse");
    document
        .select(&selector)
        .filter_map(|element| element.value().attr(attribute))
        .filter(|value| !value.chars().any(char::is_control))
        .filter_map(|value| Url::parse(value).or_else(|_| base.join(value)).ok())
        .collect()
}

fn is_in_crawl_scope(candidate: &Url, start_url: &Url, prefix_url: &Url) -> bool {
    if !matches!(candidate.scheme(), "http" | "https") {
        return false;
    }
    if candidate.scheme() != start_url.scheme()
        || candidate.host_str() != start_url.host_str()
        || candidate.port_or_known_default() != start_url.port_or_known_default()
    {
        return false;
    }
    let candidate = without_fragment(candidate).to_string();
    let prefix = without_fragment(prefix_url).to_string();
    candidate.starts_with(&prefix)
}

fn normalize_page_key(url: &Url) -> String {
    let mut normalized = without_fragment(url);
    normalized.set_query(None);
    normalized.to_string()
}

fn normalize_resource_key(url: &Url) -> String {
    let mut normalized = url.clone();
    normalized.set_fragment(None);
    normalized.to_string()
}

fn without_fragment(url: &Url) -> Url {
    let mut normalized = url.clone();
    normalized.set_fragment(None);
    normalized
}

fn is_html_like(media_type: &str) -> bool {
    let media_type = media_type
        .split(';')
        .next()
        .unwrap_or(media_type)
        .trim()
        .to_ascii_lowercase();
    matches!(
        media_type.as_str(),
        "text/html" | "application/xhtml+xml" | "application/xml" | "text/xml"
    )
}

fn default_prefix_url(source_url: &Url) -> String {
    let mut prefix = source_url.clone();
    prefix.set_query(None);
    prefix.set_fragment(None);
    if !prefix.path().ends_with('/') {
        let mut path = prefix.path().to_string();
        if let Some((parent, _)) = path.rsplit_once('/') {
            path = format!("{parent}/");
        }
        prefix.set_path(&path);
    }
    prefix.to_string()
}

fn valid_language_tag(language: &str) -> bool {
    let mut parts = language.split('-');
    let Some(primary) = parts.next() else {
        return false;
    };
    if !(2..=8).contains(&primary.len())
        || !primary
            .chars()
            .all(|character| character.is_ascii_alphabetic())
    {
        return false;
    }
    parts.all(|part| {
        (1..=8).contains(&part.len())
            && part
                .chars()
                .all(|character| character.is_ascii_alphanumeric())
    })
}

fn trim_field(value: String) -> String {
    value.trim().to_string()
}

fn is_forbidden_control(character: char) -> bool {
    character.is_control() && !matches!(character, '\n' | '\r' | '\t')
}
