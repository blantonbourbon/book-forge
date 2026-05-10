use std::{
    collections::{HashMap, HashSet, VecDeque},
    time::{Duration, Instant},
};

use url::Url;

use crate::{
    ConversionError, ConversionResult, ConversionWarning, CrawlInput, CrawlOptions, CrawlPage,
    CrawlResource,
    epub::{self, EpubResource},
    html::{self, ChapterAnalysis, ImageRewriteContext, LinkRewriteContext},
    metadata,
    text::collapse_whitespace,
    url_tools::{
        default_prefix_for, normalize_page_url, normalize_resource_url, same_origin,
        url_without_fragment,
    },
    validate_source_url,
};

struct PageSource {
    html: Option<String>,
    failure: Option<String>,
}

struct ResourceSource {
    media_type: String,
    bytes: Vec<u8>,
    failure: Option<String>,
}

struct DiscoveredPage {
    url: Url,
    key: String,
    html: String,
    analysis: ChapterAnalysis,
}

pub(crate) fn convert_crawl(input: CrawlInput) -> Result<ConversionResult, ConversionError> {
    let CrawlInput {
        start_url,
        pages,
        resources,
        metadata: raw_metadata,
        options,
        crawl,
    } = input;

    let start_url = validate_source_url(&start_url)?;
    let prefix_url = validate_prefix_url(&crawl.prefix_url, &start_url)?;
    let metadata = metadata::sanitize_metadata(raw_metadata, start_url.as_str());
    let page_lookup = build_page_lookup(pages);
    let resource_lookup = build_resource_lookup(resources);

    let mut warnings = Vec::new();
    let mut warning_keys = HashSet::new();
    let discovered_pages = discover_pages(
        &start_url,
        &prefix_url,
        &metadata,
        &crawl,
        &page_lookup,
        &mut warnings,
        &mut warning_keys,
    );

    if discovered_pages.is_empty() {
        return Err(ConversionError::NoReadableContent);
    }

    let (resources, image_paths) = if options.include_images {
        collect_image_resources(
            &discovered_pages,
            &resource_lookup,
            &mut warnings,
            &mut warning_keys,
        )
    } else {
        (Vec::new(), HashMap::new())
    };

    let chapter_paths = discovered_pages
        .iter()
        .enumerate()
        .map(|(index, page)| (page.key.clone(), format!("chapter-{}.xhtml", index + 1)))
        .collect::<HashMap<_, _>>();
    let chapter_ids = discovered_pages
        .iter()
        .map(|page| (page.key.clone(), page.analysis.ids.clone()))
        .collect::<HashMap<_, _>>();
    let link_rewrites = LinkRewriteContext {
        chapter_paths: &chapter_paths,
        chapter_ids: &chapter_ids,
    };
    let image_rewrites = ImageRewriteContext {
        packaged_paths: &image_paths,
    };

    let mut chapters = Vec::with_capacity(discovered_pages.len());
    for (index, page) in discovered_pages.iter().enumerate() {
        let chapter = html::render_chapter(
            &page.html,
            &page.url,
            &metadata,
            &options,
            index + 1,
            &page.analysis.title,
            Some(&link_rewrites),
            Some(&image_rewrites),
        )?;
        warnings.extend(chapter.warnings.iter().cloned());
        chapters.push(chapter);
    }

    let epub_bytes = epub::generate_epub(&metadata, &chapters, &resources)?;
    let download_filename = metadata::safe_download_filename(&metadata.title);

    Ok(ConversionResult {
        epub_bytes,
        download_filename,
        chapter_count: chapters.len(),
        metadata,
        warnings,
    })
}

fn discover_pages(
    start_url: &Url,
    prefix_url: &Url,
    metadata: &metadata::SanitizedMetadata,
    crawl_options: &CrawlOptions,
    page_lookup: &HashMap<String, PageSource>,
    warnings: &mut Vec<ConversionWarning>,
    warning_keys: &mut HashSet<(String, String)>,
) -> Vec<DiscoveredPage> {
    let page_limit = crawl_options.max_pages.max(1);
    let byte_limit = crawl_options.max_total_bytes.max(1);
    let time_limit = Duration::from_millis(crawl_options.max_duration_millis);
    let started = Instant::now();

    let mut discovered = Vec::new();
    let mut queued = VecDeque::new();
    let mut seen = HashSet::new();
    let start_key = normalize_page_url(start_url);
    seen.insert(start_key);
    queued.push_back((without_fragment(start_url), 0usize));
    let mut scheduled_pages = 1usize;
    let mut total_bytes = 0usize;

    while let Some((page_url, depth)) = queued.pop_front() {
        if crawl_options.max_duration_millis > 0 && started.elapsed() > time_limit {
            push_warning_once(
                warnings,
                warning_keys,
                "crawl_time_limit",
                "Crawl stopped because the configured time limit was reached.",
                None,
            );
            break;
        }

        let affected = url_without_fragment(&page_url);
        let Some(source) = page_lookup.get(&normalize_page_url(&page_url)) else {
            push_warning_once(
                warnings,
                warning_keys,
                "page_fetch_failed",
                "Page was skipped because it could not be fetched.",
                Some(affected),
            );
            continue;
        };

        if let Some(reason) = &source.failure {
            push_warning_once(
                warnings,
                warning_keys,
                "page_fetch_failed",
                format!("Page was skipped: {}", safe_warning_detail(reason)),
                Some(affected),
            );
            continue;
        }

        let Some(html) = source.html.as_ref() else {
            push_warning_once(
                warnings,
                warning_keys,
                "page_fetch_failed",
                "Page was skipped because no HTML body was available.",
                Some(affected),
            );
            continue;
        };

        if total_bytes.saturating_add(html.len()) > byte_limit {
            push_warning_once(
                warnings,
                warning_keys,
                "crawl_byte_limit",
                "Page was skipped because the configured crawl byte limit was reached.",
                Some(affected),
            );
            continue;
        }
        total_bytes += html.len();

        let analysis = match html::analyze_chapter(html, &page_url, metadata) {
            Ok(analysis) => analysis,
            Err(ConversionError::NoReadableContent) => {
                push_warning_once(
                    warnings,
                    warning_keys,
                    "page_no_readable_content",
                    "Page was skipped because it did not contain readable content.",
                    Some(affected),
                );
                continue;
            }
            Err(error) => {
                push_warning_once(
                    warnings,
                    warning_keys,
                    "page_conversion_failed",
                    error.safe_message(),
                    Some(affected),
                );
                continue;
            }
        };

        let stop_after_current_page = crawl_options.max_duration_millis == 0;
        if stop_after_current_page {
            push_warning_once(
                warnings,
                warning_keys,
                "crawl_time_limit",
                "Crawl stopped because the configured time limit was reached.",
                None,
            );
        } else {
            for raw_link in &analysis.links {
                let Some(candidate) = resolve_page_link(raw_link, &page_url) else {
                    continue;
                };

                if !is_in_scope(&candidate, start_url, prefix_url) {
                    continue;
                }

                let candidate_key = normalize_page_url(&candidate);
                if seen.contains(&candidate_key) {
                    continue;
                }

                let affected = url_without_fragment(&candidate);
                if depth.saturating_add(1) > crawl_options.max_depth {
                    push_warning_once(
                        warnings,
                        warning_keys,
                        "crawl_depth_limit",
                        "Page was skipped because the configured crawl depth limit was reached.",
                        Some(affected),
                    );
                    continue;
                }

                if scheduled_pages >= page_limit {
                    push_warning_once(
                        warnings,
                        warning_keys,
                        "crawl_page_limit",
                        "Page was skipped because the configured crawl page limit was reached.",
                        Some(affected),
                    );
                    continue;
                }

                seen.insert(candidate_key);
                scheduled_pages += 1;
                queued.push_back((without_fragment(&candidate), depth + 1));
            }
        }

        discovered.push(DiscoveredPage {
            key: normalize_page_url(&page_url),
            url: page_url,
            html: html.clone(),
            analysis,
        });

        if stop_after_current_page {
            break;
        }
    }

    discovered
}

fn collect_image_resources(
    pages: &[DiscoveredPage],
    resource_lookup: &HashMap<String, ResourceSource>,
    warnings: &mut Vec<ConversionWarning>,
    warning_keys: &mut HashSet<(String, String)>,
) -> (Vec<EpubResource>, HashMap<String, String>) {
    let mut resources = Vec::new();
    let mut packaged_paths = HashMap::new();
    let mut used_paths = HashSet::new();

    for page in pages {
        for raw_src in &page.analysis.images {
            let image_url = match resolve_image_src(raw_src, &page.url) {
                Ok(url) => url,
                Err(affected) => {
                    push_warning_once(
                        warnings,
                        warning_keys,
                        "image_unsupported_scheme",
                        "Image was skipped because its URL scheme is not supported.",
                        Some(affected),
                    );
                    continue;
                }
            };
            let key = normalize_resource_url(&image_url);
            if packaged_paths.contains_key(&key) {
                continue;
            }

            let affected = url_without_fragment(&image_url);
            let Some(source) = resource_lookup.get(&key) else {
                push_warning_once(
                    warnings,
                    warning_keys,
                    "image_fetch_failed",
                    "Image was skipped because it could not be fetched.",
                    Some(affected),
                );
                continue;
            };

            if let Some(reason) = &source.failure {
                push_warning_once(
                    warnings,
                    warning_keys,
                    "image_fetch_failed",
                    format!("Image was skipped: {}", safe_warning_detail(reason)),
                    Some(affected),
                );
                continue;
            }

            if source.bytes.is_empty() {
                push_warning_once(
                    warnings,
                    warning_keys,
                    "image_fetch_failed",
                    "Image was skipped because it had no bytes.",
                    Some(affected),
                );
                continue;
            }

            let Some((media_type, extension)) =
                supported_image_media_type(&source.media_type, &image_url)
            else {
                push_warning_once(
                    warnings,
                    warning_keys,
                    "image_unsupported_type",
                    "Image was skipped because its media type is not supported.",
                    Some(affected),
                );
                continue;
            };

            let package_path = conflict_free_resource_path(&image_url, extension, &mut used_paths);
            packaged_paths.insert(key, format!("../{package_path}"));
            resources.push(EpubResource {
                path: package_path,
                media_type,
                bytes: source.bytes.clone(),
            });
        }
    }

    (resources, packaged_paths)
}

fn build_page_lookup(pages: Vec<CrawlPage>) -> HashMap<String, PageSource> {
    let mut lookup = HashMap::new();
    for page in pages {
        let Ok(url) = validate_source_url(&page.url) else {
            continue;
        };
        lookup
            .entry(normalize_page_url(&url))
            .or_insert(PageSource {
                html: page.html,
                failure: page.failure,
            });
    }
    lookup
}

fn build_resource_lookup(resources: Vec<CrawlResource>) -> HashMap<String, ResourceSource> {
    let mut lookup = HashMap::new();
    for resource in resources {
        let Ok(url) = Url::parse(&resource.url) else {
            continue;
        };
        if !matches!(url.scheme(), "http" | "https") {
            continue;
        }
        lookup
            .entry(normalize_resource_url(&url))
            .or_insert(ResourceSource {
                media_type: resource.media_type,
                bytes: resource.bytes,
                failure: resource.failure,
            });
    }
    lookup
}

fn validate_prefix_url(raw_prefix: &str, start_url: &Url) -> Result<Url, ConversionError> {
    let prefix = if raw_prefix.trim().is_empty() {
        default_prefix_for(start_url)
    } else {
        raw_prefix.trim().to_string()
    };
    let parsed = validate_source_url(&prefix)?;
    if !same_origin(&parsed, start_url) {
        return Err(ConversionError::InvalidSourceUrl {
            message: "Crawl prefix must use the same origin as the source URL.".to_string(),
        });
    }

    if !is_in_scope(start_url, start_url, &parsed) {
        return Err(ConversionError::InvalidSourceUrl {
            message: "Source URL must be within the configured crawl prefix.".to_string(),
        });
    }

    let normalized = normalize_page_url(&parsed);
    Url::parse(&normalized).map_err(|_| ConversionError::InvalidSourceUrl {
        message: "Crawl prefix must be a valid HTTP or HTTPS URL.".to_string(),
    })
}

fn resolve_page_link(raw_href: &str, source_url: &Url) -> Option<Url> {
    let href = raw_href.trim();
    if href.is_empty() || href.starts_with('#') || href.chars().any(char::is_control) {
        return None;
    }

    let resolved = Url::parse(href).or_else(|_| source_url.join(href)).ok()?;
    matches!(resolved.scheme(), "http" | "https").then_some(resolved)
}

fn resolve_image_src(raw_src: &str, source_url: &Url) -> Result<Url, String> {
    let src = raw_src.trim();
    if src.is_empty() || src.chars().any(char::is_control) {
        return Err(src.to_string());
    }

    let resolved = Url::parse(src)
        .or_else(|_| source_url.join(src))
        .map_err(|_| src.to_string())?;
    if !matches!(resolved.scheme(), "http" | "https") {
        return Err(url_without_fragment(&resolved));
    }

    Ok(resolved)
}

fn is_in_scope(candidate: &Url, start_url: &Url, prefix_url: &Url) -> bool {
    if !same_origin(candidate, start_url) || !same_origin(candidate, prefix_url) {
        return false;
    }

    let Ok(normalized_candidate) = Url::parse(&normalize_page_url(candidate)) else {
        return false;
    };
    let Ok(normalized_prefix) = Url::parse(&normalize_page_url(prefix_url)) else {
        return false;
    };
    normalized_candidate
        .path()
        .starts_with(normalized_prefix.path())
}

fn without_fragment(url: &Url) -> Url {
    let mut url = url.clone();
    url.set_fragment(None);
    url
}

fn supported_image_media_type(raw_media_type: &str, url: &Url) -> Option<(String, &'static str)> {
    let media_type = raw_media_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();

    match media_type.as_str() {
        "image/png" => Some(("image/png".to_string(), "png")),
        "image/jpeg" | "image/jpg" => Some(("image/jpeg".to_string(), "jpg")),
        "image/gif" => Some(("image/gif".to_string(), "gif")),
        "image/webp" => Some(("image/webp".to_string(), "webp")),
        "image/svg+xml" => Some(("image/svg+xml".to_string(), "svg")),
        "" => match path_extension(url).as_deref() {
            Some("png") => Some(("image/png".to_string(), "png")),
            Some("jpg" | "jpeg") => Some(("image/jpeg".to_string(), "jpg")),
            Some("gif") => Some(("image/gif".to_string(), "gif")),
            Some("webp") => Some(("image/webp".to_string(), "webp")),
            Some("svg") => Some(("image/svg+xml".to_string(), "svg")),
            _ => None,
        },
        _ => None,
    }
}

fn conflict_free_resource_path(
    url: &Url,
    extension: &str,
    used_paths: &mut HashSet<String>,
) -> String {
    let key = normalize_resource_url(url);
    let stem = safe_resource_stem(url);
    let hash = stable_hash(&key);
    let mut path = format!("images/{stem}-{hash:016x}.{extension}");
    let mut suffix = 2usize;
    while !used_paths.insert(path.clone()) {
        path = format!("images/{stem}-{hash:016x}-{suffix}.{extension}");
        suffix += 1;
    }
    path
}

fn safe_resource_stem(url: &Url) -> String {
    let last_segment = url
        .path_segments()
        .and_then(|mut segments| segments.next_back())
        .unwrap_or("resource");
    let stem = last_segment
        .rsplit_once('.')
        .map(|(stem, _)| stem)
        .unwrap_or(last_segment);

    let mut safe = String::new();
    let mut previous_separator = false;
    for character in stem.chars().take(48) {
        if character.is_ascii_alphanumeric() {
            safe.push(character.to_ascii_lowercase());
            previous_separator = false;
        } else if !previous_separator {
            safe.push('-');
            previous_separator = true;
        }
    }

    let safe = safe.trim_matches('-').to_string();
    if safe.is_empty() {
        "resource".to_string()
    } else {
        safe
    }
}

fn path_extension(url: &Url) -> Option<String> {
    url.path_segments()
        .and_then(|mut segments| segments.next_back())
        .and_then(|segment| segment.rsplit_once('.').map(|(_, extension)| extension))
        .map(|extension| extension.to_ascii_lowercase())
}

fn stable_hash(value: &str) -> u64 {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in value.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn safe_warning_detail(raw: &str) -> String {
    let safe = raw
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect::<String>();
    let mut safe = collapse_whitespace(&safe);
    if safe.len() > 160 {
        safe.truncate(160);
    }
    safe
}

fn push_warning_once(
    warnings: &mut Vec<ConversionWarning>,
    warning_keys: &mut HashSet<(String, String)>,
    code: &str,
    message: impl Into<String>,
    affected: Option<String>,
) {
    let affected_key = affected.clone().unwrap_or_default();
    if !warning_keys.insert((code.to_string(), affected_key)) {
        return;
    }

    warnings.push(ConversionWarning {
        code: code.to_string(),
        message: message.into(),
        affected,
    });
}
