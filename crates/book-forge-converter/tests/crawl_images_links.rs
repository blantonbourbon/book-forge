use std::{
    fs,
    io::{Cursor, Read},
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use book_forge_converter::{
    BookMetadata, ConversionOptions, CrawlInput, CrawlOptions, CrawlPage, CrawlResource,
    convert_crawl,
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

fn metadata() -> BookMetadata {
    BookMetadata {
        title: "Crawl Fixture Book".to_string(),
        author: "Book Forge".to_string(),
        language: "en".to_string(),
        description: "Crawl and image fixture coverage".to_string(),
    }
}

fn page(url: &str, html: &str) -> CrawlPage {
    CrawlPage {
        url: url.to_string(),
        html: Some(html.to_string()),
        failure: None,
    }
}

fn failed_page(url: &str, reason: &str) -> CrawlPage {
    CrawlPage {
        url: url.to_string(),
        html: None,
        failure: Some(reason.to_string()),
    }
}

fn resource(url: &str, media_type: &str, bytes: Vec<u8>) -> CrawlResource {
    CrawlResource {
        url: url.to_string(),
        media_type: media_type.to_string(),
        bytes,
        failure: None,
    }
}

fn inspect_bytes(bytes: &[u8]) -> book_forge_epub_inspector::InspectionReport {
    let path = std::env::temp_dir().join(format!(
        "book-forge-crawl-{}.epub",
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

fn zip_text(bytes: &[u8], path: &str) -> String {
    let mut archive = ZipArchive::new(Cursor::new(bytes)).expect("epub should be a zip archive");
    let mut entry = String::new();
    archive
        .by_name(path)
        .unwrap_or_else(|_| panic!("{path} should be present"))
        .read_to_string(&mut entry)
        .expect("zip entry should be utf-8");
    entry
}

fn crawl_graph_input(max_depth: usize, max_pages: usize) -> CrawlInput {
    CrawlInput {
        start_url: "https://example.test/crawl-graph/index.html".to_string(),
        pages: vec![
            page(
                "https://example.test/crawl-graph/index.html",
                &fixture("html/crawl-graph/index.html"),
            ),
            page(
                "https://example.test/crawl-graph/chapter-one.html",
                &fixture("html/crawl-graph/chapter-one.html"),
            ),
            page(
                "https://example.test/crawl-graph/chapter-two.html",
                &fixture("html/crawl-graph/chapter-two.html"),
            ),
            page(
                "https://example.test/crawl-graph/deep/chapter-three.html",
                &fixture("html/crawl-graph/deep/chapter-three.html"),
            ),
            page(
                "https://example.test/single-page/index.html",
                &fixture("html/single-page/index.html"),
            ),
        ],
        resources: Vec::new(),
        metadata: metadata(),
        options: ConversionOptions {
            include_images: false,
        },
        crawl: CrawlOptions {
            prefix_url: "https://example.test/crawl-graph/".to_string(),
            max_depth,
            max_pages,
            ..CrawlOptions::default()
        },
    }
}

#[test]
fn crawl_respects_prefix_order_duplicates_and_rewrites_links() {
    let first = convert_crawl(crawl_graph_input(2, 10)).expect("crawl conversion should succeed");
    let second = convert_crawl(crawl_graph_input(2, 10)).expect("crawl conversion should repeat");

    assert_eq!(first.chapter_count, 4);
    assert!(first.warnings.is_empty());

    let first_report = inspect_bytes(&first.epub_bytes);
    let second_report = inspect_bytes(&second.epub_bytes);
    assert!(
        first_report.ok,
        "inspection errors: {:?}",
        first_report.errors
    );
    assert!(
        second_report.ok,
        "inspection errors: {:?}",
        second_report.errors
    );

    let first_package = first_report.package.expect("package should inspect");
    let second_package = second_report.package.expect("package should inspect");
    let first_titles = first_package
        .nav_entries
        .iter()
        .map(|entry| entry.label.as_str())
        .collect::<Vec<_>>();
    let second_titles = second_package
        .nav_entries
        .iter()
        .map(|entry| entry.label.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        first_titles,
        vec!["Crawl Start", "Chapter One", "Chapter Two", "Chapter Three"]
    );
    assert_eq!(first_titles, second_titles);

    let start = zip_text(&first.epub_bytes, "EPUB/chapters/chapter-1.xhtml");
    assert!(start.contains("marker-crawl-001: start page must be first"));
    assert!(start.contains("href=\"chapter-2.xhtml\""));
    assert!(start.contains("href=\"chapter-3.xhtml#section-two\""));
    assert!(start.contains("href=\"chapter-2.xhtml#duplicate\""));
    assert!(start.contains("href=\"chapter-4.xhtml\""));
    assert!(start.contains("href=\"https://example.test/single-page/index.html\""));
    assert!(start.contains("href=\"https://example.org/external.html\""));

    let all_chapters = (1..=4)
        .map(|index| {
            zip_text(
                &first.epub_bytes,
                &format!("EPUB/chapters/chapter-{index}.xhtml"),
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(all_chapters.matches("marker-crawl-101").count(), 1);
    assert_eq!(all_chapters.matches("marker-crawl-201").count(), 1);
    assert!(!all_chapters.contains("marker-single-001"));
}

#[test]
fn crawl_limits_and_recoverable_page_failures_emit_warnings() {
    let page_limited =
        convert_crawl(crawl_graph_input(2, 2)).expect("limited crawl should succeed");
    assert_eq!(page_limited.chapter_count, 2);
    assert!(page_limited.warnings.iter().any(|warning| {
        warning.code == "crawl_page_limit"
            && warning.affected.as_deref()
                == Some("https://example.test/crawl-graph/chapter-two.html")
    }));

    let depth_limited =
        convert_crawl(crawl_graph_input(0, 10)).expect("depth crawl should succeed");
    assert_eq!(depth_limited.chapter_count, 1);
    assert!(depth_limited.warnings.iter().any(|warning| {
        warning.code == "crawl_depth_limit"
            && warning.affected.as_deref()
                == Some("https://example.test/crawl-graph/chapter-one.html")
    }));

    let mut byte_limited_input = crawl_graph_input(2, 10);
    byte_limited_input.crawl.max_total_bytes = fixture("html/crawl-graph/index.html").len() + 1;
    let byte_limited =
        convert_crawl(byte_limited_input).expect("byte-limited crawl should succeed");
    assert_eq!(byte_limited.chapter_count, 1);
    assert!(byte_limited.warnings.iter().any(|warning| {
        warning.code == "crawl_byte_limit"
            && warning.affected.as_deref()
                == Some("https://example.test/crawl-graph/chapter-one.html")
    }));

    let mut time_limited_input = crawl_graph_input(2, 10);
    time_limited_input.crawl.max_duration_millis = 0;
    let time_limited =
        convert_crawl(time_limited_input).expect("time-limited crawl should succeed");
    assert_eq!(time_limited.chapter_count, 1);
    assert!(
        time_limited
            .warnings
            .iter()
            .any(|warning| warning.code == "crawl_time_limit" && warning.affected.is_none())
    );

    let failed = convert_crawl(CrawlInput {
        start_url: "https://example.test/failed-resources/index.html".to_string(),
        pages: vec![page(
            "https://example.test/failed-resources/index.html",
            &fixture("html/failed-resources/index.html"),
        )],
        resources: Vec::new(),
        metadata: metadata(),
        options: ConversionOptions {
            include_images: false,
        },
        crawl: CrawlOptions {
            prefix_url: "https://example.test/failed-resources/".to_string(),
            max_depth: 1,
            max_pages: 5,
            ..CrawlOptions::default()
        },
    })
    .expect("missing linked page should be recoverable");
    assert_eq!(failed.chapter_count, 1);
    assert!(failed.warnings.iter().any(|warning| {
        warning.code == "page_fetch_failed"
            && warning.affected.as_deref()
                == Some("https://example.test/failed-resources/missing-page.html")
    }));

    let explicit_failed = convert_crawl(CrawlInput {
        start_url: "https://example.test/crawl-graph/index.html".to_string(),
        pages: vec![
            page(
                "https://example.test/crawl-graph/index.html",
                &fixture("html/crawl-graph/index.html"),
            ),
            failed_page(
                "https://example.test/crawl-graph/chapter-one.html",
                "fixture returned 503",
            ),
        ],
        resources: Vec::new(),
        metadata: metadata(),
        options: ConversionOptions {
            include_images: false,
        },
        crawl: CrawlOptions {
            prefix_url: "https://example.test/crawl-graph/".to_string(),
            max_depth: 1,
            max_pages: 5,
            ..CrawlOptions::default()
        },
    })
    .expect("explicit failed page should be recoverable when start succeeds");
    assert!(explicit_failed.warnings.iter().any(|warning| {
        warning.code == "page_fetch_failed"
            && warning.message.contains("fixture returned 503")
            && warning.affected.as_deref()
                == Some("https://example.test/crawl-graph/chapter-one.html")
    }));
}

#[test]
fn crawl_embeds_images_with_deterministic_conflict_free_paths() {
    let start_html = r#"
        <!doctype html>
        <html><head><title>Image Start</title></head><body><article>
          <h1>Image Start</h1>
          <p>marker-image-crawl-001: start with images.</p>
          <img src="/images/logo.svg" alt="Book Forge logo fixture" />
          <img src="/images/diagram.svg?variant=one&amp;name=special%20chars" alt="Diagram with query" />
          <a href="./second.html">Second image page</a>
        </article></body></html>
    "#;
    let second_html = r#"
        <!doctype html>
        <html><head><title>Image Second</title></head><body><article>
          <h1>Image Second</h1>
          <p>marker-image-crawl-002: shared image should not duplicate.</p>
          <img src="/images/logo.svg" alt="Shared logo again" />
        </article></body></html>
    "#;

    let input = || CrawlInput {
        start_url: "https://example.test/images-crawl/index.html".to_string(),
        pages: vec![
            page("https://example.test/images-crawl/index.html", start_html),
            page("https://example.test/images-crawl/second.html", second_html),
        ],
        resources: vec![
            resource(
                "https://example.test/images/logo.svg",
                "image/svg+xml",
                fixture_bytes("images/logo.svg"),
            ),
            resource(
                "https://example.test/images/diagram.svg?variant=one&name=special%20chars",
                "image/svg+xml",
                fixture_bytes("images/diagram.svg"),
            ),
        ],
        metadata: metadata(),
        options: ConversionOptions {
            include_images: true,
        },
        crawl: CrawlOptions {
            prefix_url: "https://example.test/images-crawl/".to_string(),
            max_depth: 1,
            max_pages: 5,
            ..CrawlOptions::default()
        },
    };

    let first = convert_crawl(input()).expect("image crawl should succeed");
    let second = convert_crawl(input()).expect("image crawl should be deterministic");
    assert!(first.warnings.is_empty());

    let first_report = inspect_bytes(&first.epub_bytes);
    let second_report = inspect_bytes(&second.epub_bytes);
    assert!(
        first_report.ok,
        "inspection errors: {:?}",
        first_report.errors
    );
    assert!(
        second_report.ok,
        "inspection errors: {:?}",
        second_report.errors
    );
    let first_entries = first_report.entries;
    let second_entries = second_report.entries;
    assert_eq!(first_entries, second_entries);

    let image_entries = first_entries
        .iter()
        .filter(|entry| entry.starts_with("EPUB/images/"))
        .collect::<Vec<_>>();
    assert_eq!(image_entries.len(), 2);
    for entry in image_entries {
        assert!(!entry.contains('?'));
        assert!(!entry.contains(' '));
        assert!(!entry.contains(".."));
    }

    let start = zip_text(&first.epub_bytes, "EPUB/chapters/chapter-1.xhtml");
    let second_chapter = zip_text(&first.epub_bytes, "EPUB/chapters/chapter-2.xhtml");
    assert!(start.contains("<img src=\"../images/"));
    assert!(second_chapter.contains("<img src=\"../images/"));
    assert_eq!(
        start.matches("Book Forge logo fixture").count()
            + second_chapter.matches("Shared logo again").count(),
        2
    );
    assert!(start.contains("alt=\"Diagram with query\""));
    assert!(!start.contains("src=\"https://"));
}

#[test]
fn include_images_off_and_failed_images_leave_no_broken_references() {
    let disabled = convert_crawl(CrawlInput {
        start_url: "https://example.test/html/images/index.html".to_string(),
        pages: vec![page(
            "https://example.test/html/images/index.html",
            &fixture("html/images/index.html"),
        )],
        resources: vec![
            resource(
                "https://example.test/images/logo.svg",
                "image/svg+xml",
                fixture_bytes("images/logo.svg"),
            ),
            resource(
                "https://example.test/images/diagram.svg?variant=one",
                "image/svg+xml",
                fixture_bytes("images/diagram.svg"),
            ),
        ],
        metadata: metadata(),
        options: ConversionOptions {
            include_images: false,
        },
        crawl: CrawlOptions {
            prefix_url: "https://example.test/html/images/".to_string(),
            max_depth: 0,
            max_pages: 5,
            ..CrawlOptions::default()
        },
    })
    .expect("disabled-image crawl should succeed");
    let disabled_report = inspect_bytes(&disabled.epub_bytes);
    assert!(
        disabled_report.ok,
        "inspection errors: {:?}",
        disabled_report.errors
    );
    let package = disabled_report.package.expect("package should inspect");
    assert!(
        !package
            .manifest
            .iter()
            .any(|item| item.media_type.starts_with("image/"))
    );
    let disabled_chapter = zip_text(&disabled.epub_bytes, "EPUB/chapters/chapter-1.xhtml");
    assert!(!disabled_chapter.contains("<img "));
    assert!(disabled_chapter.contains("Book Forge logo fixture"));

    let failed_images = convert_crawl(CrawlInput {
        start_url: "https://example.test/failed-resources/index.html".to_string(),
        pages: vec![page(
            "https://example.test/failed-resources/index.html",
            &fixture("html/failed-resources/index.html"),
        )],
        resources: Vec::new(),
        metadata: metadata(),
        options: ConversionOptions {
            include_images: true,
        },
        crawl: CrawlOptions {
            prefix_url: "https://example.test/failed-resources/".to_string(),
            max_depth: 1,
            max_pages: 5,
            ..CrawlOptions::default()
        },
    })
    .expect("failed images should be recoverable");
    assert!(failed_images.warnings.iter().any(|warning| {
        warning.code == "image_fetch_failed"
            && warning.affected.as_deref()
                == Some("https://example.test/failed-resources/missing-image.png")
    }));
    assert!(failed_images.warnings.iter().any(|warning| {
        warning.code == "image_unsupported_scheme"
            && warning.affected.as_deref() == Some("ftp://example.org/not-supported.png")
    }));
    let failed_report = inspect_bytes(&failed_images.epub_bytes);
    assert!(
        failed_report.ok,
        "inspection errors: {:?}",
        failed_report.errors
    );
    let failed_chapter = zip_text(&failed_images.epub_bytes, "EPUB/chapters/chapter-1.xhtml");
    assert!(!failed_chapter.contains("missing-image.png"));
    assert!(!failed_chapter.contains("ftp://example.org/not-supported.png"));
    assert!(failed_chapter.contains("Missing image fixture"));
    assert!(failed_chapter.contains("Unsupported image scheme"));
}
