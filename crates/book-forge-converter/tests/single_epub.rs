use std::{
    fs,
    io::{Cursor, Read},
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use book_forge_converter::{
    BookMetadata, ConversionError, ConversionOptions, SinglePageInput, convert_single_page,
};
use book_forge_epub_inspector::inspect_epub;
use zip::ZipArchive;

fn fixture(path: &str) -> String {
    fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures")
            .join(path),
    )
    .expect("fixture should be readable")
}

fn convert_fixture(path: &str, metadata: BookMetadata) -> book_forge_converter::ConversionResult {
    convert_single_page(SinglePageInput {
        source_url: "https://example.test/book/index.html".to_string(),
        html: fixture(path),
        metadata,
        options: ConversionOptions::default(),
    })
    .expect("fixture conversion should succeed")
}

fn metadata_fixture() -> BookMetadata {
    BookMetadata {
        title: "  <script>ignored()</script> One / Great Book ..\r\n  ".to_string(),
        author: "  <b>Ada</b>\u{0007} Lovelace  ".to_string(),
        language: " en-US ".to_string(),
        description: "  <img src=x onerror=bad()> Description & details\u{0008}  ".to_string(),
    }
}

fn inspect_bytes(bytes: &[u8]) -> book_forge_epub_inspector::InspectionReport {
    let path = std::env::temp_dir().join(format!(
        "book-forge-single-{}.epub",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos()
    ));
    fs::write(&path, bytes).expect("epub bytes should be writable for inspection");
    let report = inspect_epub(&path);
    fs::remove_file(path).expect("temporary epub should be removed");
    report
}

fn chapter_xhtml(bytes: &[u8]) -> String {
    let mut archive = ZipArchive::new(Cursor::new(bytes)).expect("epub should be a zip archive");
    let mut chapter = String::new();
    archive
        .by_name("EPUB/chapters/chapter-1.xhtml")
        .expect("single chapter should be present")
        .read_to_string(&mut chapter)
        .expect("chapter xhtml should be utf-8");
    chapter
}

fn assert_markers_in_order(text: &str, markers: &[&str]) {
    let mut last = 0;
    for marker in markers {
        let offset = text[last..]
            .find(marker)
            .unwrap_or_else(|| panic!("missing marker {marker:?} in {text}"));
        last += offset;
    }
}

#[test]
fn single_page_epub_contains_sanitized_metadata_and_one_chapter() {
    let result = convert_fixture("html/single-page/index.html", metadata_fixture());

    assert_eq!(result.chapter_count, 1);
    assert!(result.warnings.is_empty());
    assert!(result.download_filename.ends_with(".epub"));
    assert!(!result.download_filename.contains('/'));
    assert!(!result.download_filename.contains('\\'));
    assert!(!result.download_filename.contains(".."));
    assert!(!result.download_filename.contains(['\r', '\n']));
    assert!(!result.download_filename.ends_with(".html"));

    let report = inspect_bytes(&result.epub_bytes);
    assert!(report.ok, "inspection errors: {:?}", report.errors);
    assert!(report.required_entries.mimetype);
    assert!(report.required_entries.container_xml);
    assert!(report.required_entries.package_document);
    assert!(report.required_entries.navigation_document);

    let package = report.package.expect("package report should be populated");
    assert_eq!(package.metadata.title, "ignored() One Great Book");
    assert_eq!(package.metadata.author, "Ada Lovelace");
    assert_eq!(package.metadata.language, "en-US");
    assert_eq!(package.metadata.description, "Description & details");
    assert!(!package.metadata.identifier.is_empty());
    assert!(!package.metadata.modified.is_empty());
    assert_eq!(package.content_chapters.len(), 1);
    assert_eq!(package.nav_entries.len(), 1);
    assert_eq!(
        package.nav_entries[0].href,
        package.content_chapters[0].href
    );

    let chapter = chapter_xhtml(&result.epub_bytes);
    assert!(chapter.contains("<title>Single Page Fixture</title>"));
    assert_markers_in_order(
        &chapter,
        &[
            "marker-single-001: first paragraph",
            "marker-single-002: second paragraph",
        ],
    );
}

#[test]
fn sanitization_removes_active_content_and_preserves_semantic_reading_order() {
    let unsafe_result = convert_fixture("html/unsafe-html/index.html", metadata_fixture());
    let unsafe_chapter = chapter_xhtml(&unsafe_result.epub_bytes).to_lowercase();

    for forbidden in [
        "<script",
        "onload=",
        "onclick=",
        "javascript:",
        "<form",
        "<iframe",
        "<object",
        "<embed",
    ] {
        assert!(
            !unsafe_chapter.contains(forbidden),
            "chapter should not contain {forbidden}"
        );
    }
    assert!(unsafe_chapter.contains("marker-unsafe-001: safe text should remain readable"));

    let semantic_result = convert_fixture("html/semantic-content/index.html", metadata_fixture());
    let semantic_chapter = chapter_xhtml(&semantic_result.epub_bytes);
    assert_markers_in_order(
        &semantic_chapter,
        &[
            "marker-semantic-001",
            "marker-semantic-002",
            "marker-semantic-003",
            "marker-semantic-004",
            "marker-semantic-005",
            "marker-semantic-006",
            "marker-semantic-007",
            "marker-semantic-008",
            "marker-semantic-009",
        ],
    );
    for preserved_tag in [
        "<blockquote>",
        "<ol>",
        "<ul>",
        "<pre>",
        "<code>",
        "<table>",
        "<th",
    ] {
        assert!(
            semantic_chapter.contains(preserved_tag),
            "semantic tag {preserved_tag} should be preserved"
        );
    }
}

#[test]
fn unsafe_internal_references_are_neutralized_and_resolvable_links_survive() {
    let html = r##"
        <!doctype html>
        <html lang="en">
          <head><title>Reference Fixture</title></head>
          <body>
            <article>
              <h1>Reference Fixture</h1>
              <p id="section">marker-ref-001: target paragraph.</p>
              <a href="#section">Jump to target</a>
              <a href="./next.html">Relative page that is not in the EPUB</a>
              <a href="/absolute.html">Absolute page that is not in the EPUB</a>
              <a href="https://example.org/permitted">Permitted external link</a>
              <a href="javascript:alert(1)">Unsafe script link</a>
              <img src="./missing.png" alt="Missing image alt text" />
            </article>
          </body>
        </html>
    "##;

    let result = convert_single_page(SinglePageInput {
        source_url: "https://example.test/book/index.html".to_string(),
        html: html.to_string(),
        metadata: BookMetadata {
            title: "Reference Fixture".to_string(),
            author: "Book Forge".to_string(),
            language: "en".to_string(),
            description: "Reference test".to_string(),
        },
        options: ConversionOptions::default(),
    })
    .expect("reference fixture should convert");
    let report = inspect_bytes(&result.epub_bytes);
    assert!(report.ok, "inspection errors: {:?}", report.errors);

    let chapter = chapter_xhtml(&result.epub_bytes);
    assert!(chapter.contains("href=\"#section\""));
    assert!(chapter.contains("href=\"https://example.org/permitted\""));
    assert!(!chapter.contains("./next.html"));
    assert!(!chapter.contains("/absolute.html"));
    assert!(!chapter.contains("javascript:"));
    assert!(!chapter.contains("./missing.png"));
    assert!(chapter.contains("Missing image alt text"));
}

#[test]
fn fatal_failures_return_structured_errors_instead_of_epub_bytes() {
    let invalid_url = convert_single_page(SinglePageInput {
        source_url: "file:///etc/passwd".to_string(),
        html: fixture("html/single-page/index.html"),
        metadata: metadata_fixture(),
        options: ConversionOptions::default(),
    })
    .expect_err("file urls must fail before EPUB generation");
    assert!(matches!(
        invalid_url,
        ConversionError::InvalidSourceUrl { .. }
    ));
    assert_eq!(invalid_url.code(), "invalid_source_url");
    assert!(!invalid_url.safe_message().contains("/home/"));

    let no_content = convert_single_page(SinglePageInput {
        source_url: "https://example.test/empty.html".to_string(),
        html: "<html><body><script>alert(1)</script><form><input /></form></body></html>"
            .to_string(),
        metadata: metadata_fixture(),
        options: ConversionOptions::default(),
    })
    .expect_err("active-only pages should not produce corrupt EPUBs");
    assert!(matches!(no_content, ConversionError::NoReadableContent));
    assert_eq!(no_content.code(), "no_readable_content");
    assert!(no_content.safe_message().contains("readable"));
}
