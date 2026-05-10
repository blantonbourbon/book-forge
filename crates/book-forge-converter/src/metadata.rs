use serde::{Deserialize, Serialize};

use crate::{BookMetadata, text::collapse_whitespace, text::strip_html_tags};

const MODIFIED_DATE: &str = "2026-05-10T00:00:00Z";

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct SanitizedMetadata {
    pub title: String,
    pub author: String,
    pub language: String,
    pub description: String,
    pub identifier: String,
    pub modified: String,
}

pub(crate) fn sanitize_metadata(metadata: BookMetadata, source_url: &str) -> SanitizedMetadata {
    let title = sanitize_metadata_value(&metadata.title, "Untitled Book");
    let author = sanitize_metadata_value(&metadata.author, "Unknown Author");
    let language = sanitize_language(&metadata.language);
    let description = sanitize_metadata_value(&metadata.description, "");
    let identifier = stable_identifier(source_url, &title);

    SanitizedMetadata {
        title,
        author,
        language,
        description,
        identifier,
        modified: MODIFIED_DATE.to_string(),
    }
}

pub(crate) fn sanitize_metadata_value(raw: &str, fallback: &str) -> String {
    let without_tags = strip_html_tags(raw);
    let mut cleaned = String::with_capacity(without_tags.len());

    for character in without_tags.chars() {
        if character.is_control()
            || matches!(
                character,
                '/' | '\\' | '\u{2028}' | '\u{2029}' | '<' | '>' | '"' | '\''
            )
        {
            cleaned.push(' ');
        } else {
            cleaned.push(character);
        }
    }

    while cleaned.contains("..") {
        cleaned = cleaned.replace("..", " ");
    }

    let collapsed = collapse_whitespace(&cleaned);
    if collapsed.is_empty() {
        fallback.to_string()
    } else {
        collapsed
    }
}

pub(crate) fn safe_download_filename(title: &str) -> String {
    let mut filename = String::with_capacity(title.len() + 5);
    let mut previous_separator = false;

    for character in title.chars() {
        if character.is_ascii_alphanumeric() {
            filename.push(character.to_ascii_lowercase());
            previous_separator = false;
        } else if !previous_separator {
            filename.push('-');
            previous_separator = true;
        }

        if filename.len() >= 80 {
            break;
        }
    }

    let filename = filename.trim_matches('-').trim_matches('.').to_string();
    let filename = if filename.is_empty() {
        "book-forge".to_string()
    } else {
        filename
    };

    format!("{filename}.epub")
}

fn sanitize_language(raw: &str) -> String {
    let candidate: String = raw
        .trim()
        .chars()
        .filter(|character| character.is_ascii_alphanumeric() || *character == '-')
        .take(35)
        .collect();

    if candidate.is_empty() {
        "en".to_string()
    } else {
        candidate
    }
}

fn stable_identifier(source_url: &str, title: &str) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in source_url.bytes().chain(title.bytes()) {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("urn:book-forge:{hash:016x}")
}
