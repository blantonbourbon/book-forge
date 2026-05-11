use std::{
    fs,
    io::{Cursor, Read},
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use book_forge_converter::{
    BookMetadata, ConversionError, ConversionOptions, CrawlResource, SinglePageInput,
    convert_single_page,
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

fn fixture_bytes(path: &str) -> Vec<u8> {
    fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures")
            .join(path),
    )
    .expect("fixture bytes should be readable")
}

fn convert_fixture(path: &str, metadata: BookMetadata) -> book_forge_converter::ConversionResult {
    convert_single_page(SinglePageInput {
        source_url: "https://example.test/book/index.html".to_string(),
        html: fixture(path),
        resources: Vec::new(),
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

fn image_resource(url: &str, media_type: &str, bytes: Vec<u8>) -> CrawlResource {
    CrawlResource {
        url: url.to_string(),
        media_type: media_type.to_string(),
        bytes,
        failure: None,
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
        resources: Vec::new(),
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
fn absolute_same_document_links_rewrite_existing_fragments_and_neutralize_missing_anchors() {
    let html = r##"
        <!doctype html>
        <html lang="en">
          <head><title>Same Document Fixture</title></head>
          <body>
            <article>
              <h1>Same Document Fixture</h1>
              <p id="section">marker-same-doc-001: target paragraph.</p>
              <a href="https://example.test/book/index.html#section">Absolute same-document target</a>
              <a href="https://example.test/book/index.html#missing">Absolute missing target</a>
              <a href="https://example.org/permitted">Permitted external link</a>
            </article>
          </body>
        </html>
    "##;

    let result = convert_single_page(SinglePageInput {
        source_url: "https://example.test/book/index.html".to_string(),
        html: html.to_string(),
        resources: Vec::new(),
        metadata: BookMetadata {
            title: "Same Document Fixture".to_string(),
            author: "Book Forge".to_string(),
            language: "en".to_string(),
            description: "Absolute same-document link regression".to_string(),
        },
        options: ConversionOptions::default(),
    })
    .expect("same-document fixture should convert");
    let report = inspect_bytes(&result.epub_bytes);
    assert!(report.ok, "inspection errors: {:?}", report.errors);

    let chapter = chapter_xhtml(&result.epub_bytes);
    assert!(chapter.contains("href=\"#section\""));
    assert!(chapter.contains("Absolute same-document target"));
    assert!(chapter.contains("Absolute missing target"));
    assert!(chapter.contains("href=\"https://example.org/permitted\""));
    assert!(!chapter.contains("https://example.test/book/index.html#section"));
    assert!(!chapter.contains("https://example.test/book/index.html#missing"));
    assert!(!chapter.contains("href=\"#missing\""));
}

#[test]
fn single_page_include_images_embeds_supplied_images_and_disabled_mode_has_no_broken_refs() {
    let html = r#"
        <!doctype html>
        <html lang="en">
          <head><title>Single Image Fixture</title></head>
          <body>
            <article>
              <h1>Single Image Fixture</h1>
              <p>marker-single-image-001: page with one supplied image.</p>
              <img src="/images/logo.svg" alt="Book Forge logo fixture" />
            </article>
          </body>
        </html>
    "#;
    let input = |include_images| SinglePageInput {
        source_url: "https://example.test/html/images/index.html".to_string(),
        html: html.to_string(),
        resources: vec![image_resource(
            "https://example.test/images/logo.svg",
            "image/svg+xml",
            fixture_bytes("images/logo.svg"),
        )],
        metadata: BookMetadata {
            title: "Single Image Fixture".to_string(),
            author: "Book Forge".to_string(),
            language: "en".to_string(),
            description: "Single-page image option regression".to_string(),
        },
        options: ConversionOptions { include_images },
    };

    let embedded = convert_single_page(input(true)).expect("enabled images should convert");
    let embedded_report = inspect_bytes(&embedded.epub_bytes);
    assert!(
        embedded_report.ok,
        "inspection errors: {:?}",
        embedded_report.errors
    );
    let embedded_package = embedded_report.package.expect("package should inspect");
    assert!(
        embedded_package
            .manifest
            .iter()
            .any(|item| item.media_type == "image/svg+xml")
    );
    assert!(
        embedded_report
            .entries
            .iter()
            .any(|entry| entry.starts_with("EPUB/images/") && entry.ends_with(".svg"))
    );
    let embedded_chapter = chapter_xhtml(&embedded.epub_bytes);
    assert!(embedded_chapter.contains("<img src=\"../images/"));
    assert!(embedded_chapter.contains("alt=\"Book Forge logo fixture\""));
    assert!(!embedded_chapter.contains("src=\"https://"));

    let disabled = convert_single_page(input(false)).expect("disabled images should convert");
    let disabled_report = inspect_bytes(&disabled.epub_bytes);
    assert!(
        disabled_report.ok,
        "inspection errors: {:?}",
        disabled_report.errors
    );
    let disabled_package = disabled_report.package.expect("package should inspect");
    assert!(
        !disabled_package
            .manifest
            .iter()
            .any(|item| item.media_type.starts_with("image/"))
    );
    assert!(
        !disabled_report
            .entries
            .iter()
            .any(|entry| entry.starts_with("EPUB/images/"))
    );
    let disabled_chapter = chapter_xhtml(&disabled.epub_bytes);
    assert!(!disabled_chapter.contains("<img "));
    assert!(!disabled_chapter.contains("/images/logo.svg"));
    assert!(disabled_chapter.contains("Book Forge logo fixture"));
}

#[test]
fn fatal_failures_return_structured_errors_instead_of_epub_bytes() {
    let invalid_url = convert_single_page(SinglePageInput {
        source_url: "file:///etc/passwd".to_string(),
        html: fixture("html/single-page/index.html"),
        resources: Vec::new(),
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
        resources: Vec::new(),
        metadata: metadata_fixture(),
        options: ConversionOptions::default(),
    })
    .expect_err("active-only pages should not produce corrupt EPUBs");
    assert!(matches!(no_content, ConversionError::NoReadableContent));
    assert_eq!(no_content.code(), "no_readable_content");
    assert!(no_content.safe_message().contains("readable"));
}
