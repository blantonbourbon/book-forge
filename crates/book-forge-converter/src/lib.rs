mod crawl;
mod epub;
mod html;
mod metadata;
mod text;
mod url_tools;

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use url::Url;
use zip::result::ZipError;

pub use metadata::SanitizedMetadata;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub enum ConversionMode {
    Single,
    Crawl,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
pub struct BookMetadata {
    pub title: String,
    pub author: String,
    pub language: String,
    pub description: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
pub struct ConversionOptions {
    pub include_images: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct CrawlOptions {
    pub prefix_url: String,
    pub max_depth: usize,
    pub max_pages: usize,
    pub max_total_bytes: usize,
    pub max_duration_millis: u64,
}

impl Default for CrawlOptions {
    fn default() -> Self {
        Self {
            prefix_url: String::new(),
            max_depth: 3,
            max_pages: 50,
            max_total_bytes: 10 * 1024 * 1024,
            max_duration_millis: 30_000,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct SinglePageInput {
    pub source_url: String,
    pub html: String,
    #[serde(default)]
    pub resources: Vec<CrawlResource>,
    pub metadata: BookMetadata,
    pub options: ConversionOptions,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct CrawlPage {
    pub url: String,
    pub html: Option<String>,
    pub failure: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct CrawlResource {
    pub url: String,
    pub media_type: String,
    pub bytes: Vec<u8>,
    pub failure: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct CrawlInput {
    pub start_url: String,
    pub pages: Vec<CrawlPage>,
    pub resources: Vec<CrawlResource>,
    pub metadata: BookMetadata,
    pub options: ConversionOptions,
    pub crawl: CrawlOptions,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct ConversionResult {
    pub epub_bytes: Vec<u8>,
    pub download_filename: String,
    pub chapter_count: usize,
    pub metadata: SanitizedMetadata,
    pub warnings: Vec<ConversionWarning>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct ConversionWarning {
    pub code: String,
    pub message: String,
    pub affected: Option<String>,
}

#[derive(Debug, Error)]
pub enum ConversionError {
    #[error("invalid source URL")]
    InvalidSourceUrl { message: String },
    #[error("no readable content")]
    NoReadableContent,
    #[error("could not generate EPUB")]
    EpubGeneration { message: String },
}

impl ConversionError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidSourceUrl { .. } => "invalid_source_url",
            Self::NoReadableContent => "no_readable_content",
            Self::EpubGeneration { .. } => "epub_generation_failed",
        }
    }

    pub fn safe_message(&self) -> String {
        match self {
            Self::InvalidSourceUrl { message } => message.clone(),
            Self::NoReadableContent => {
                "The page did not contain readable content after sanitization.".to_string()
            }
            Self::EpubGeneration { .. } => {
                "The EPUB could not be generated from the supplied content.".to_string()
            }
        }
    }
}

impl From<ZipError> for ConversionError {
    fn from(error: ZipError) -> Self {
        Self::EpubGeneration {
            message: error.to_string(),
        }
    }
}

impl From<std::io::Error> for ConversionError {
    fn from(error: std::io::Error) -> Self {
        Self::EpubGeneration {
            message: error.to_string(),
        }
    }
}

pub fn boundary_name() -> &'static str {
    "converter"
}

pub fn convert_single_page(input: SinglePageInput) -> Result<ConversionResult, ConversionError> {
    let source_url = validate_source_url(&input.source_url)?;
    let metadata = metadata::sanitize_metadata(input.metadata, source_url.as_str());
    let analysis = html::analyze_chapter(&input.html, &source_url, &metadata)?;

    let mut warnings = Vec::new();
    let mut warning_keys = HashSet::new();
    let (resources, image_paths) = if input.options.include_images {
        let resource_lookup = crawl::build_resource_lookup(input.resources);
        crawl::collect_image_resources(
            &[crawl::ImageSource {
                url: &source_url,
                images: &analysis.images,
            }],
            &resource_lookup,
            &mut warnings,
            &mut warning_keys,
        )
    } else {
        (Vec::new(), Default::default())
    };

    let image_rewrites = html::ImageRewriteContext {
        packaged_paths: &image_paths,
    };
    let chapter = html::render_chapter(
        &input.html,
        &source_url,
        &metadata,
        &input.options,
        1,
        &analysis.title,
        None,
        Some(&image_rewrites),
    )?;
    warnings.extend(chapter.warnings.iter().cloned());
    let epub_bytes = epub::generate_single_epub(&metadata, &chapter, &resources)?;
    let download_filename = metadata::safe_download_filename(&metadata.title);

    Ok(ConversionResult {
        epub_bytes,
        download_filename,
        chapter_count: 1,
        metadata,
        warnings,
    })
}

pub fn convert_crawl(input: CrawlInput) -> Result<ConversionResult, ConversionError> {
    crawl::convert_crawl(input)
}

pub(crate) fn validate_source_url(source_url: &str) -> Result<Url, ConversionError> {
    let parsed = Url::parse(source_url).map_err(|_| ConversionError::InvalidSourceUrl {
        message: "Source URL must be an absolute HTTP or HTTPS URL.".to_string(),
    })?;

    match parsed.scheme() {
        "http" | "https" => Ok(parsed),
        _ => Err(ConversionError::InvalidSourceUrl {
            message: "Source URL must use HTTP or HTTPS.".to_string(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::{ConversionMode, boundary_name, metadata::sanitize_metadata_value};

    #[test]
    fn exposes_converter_boundary_name() {
        assert_eq!(boundary_name(), "converter");
    }

    #[test]
    fn declares_single_and_crawl_modes() {
        assert_ne!(ConversionMode::Single, ConversionMode::Crawl);
    }

    #[test]
    fn metadata_sanitization_removes_markup_controls_and_path_markers() {
        assert_eq!(
            sanitize_metadata_value(" <b>Ada</b>\u{0007} / Lovelace .. ", "fallback"),
            "Ada Lovelace"
        );
        assert_eq!(sanitize_metadata_value(" \n\t ", "fallback"), "fallback");
    }
}
